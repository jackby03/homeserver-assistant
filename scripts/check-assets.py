#!/usr/bin/env python3
"""Static validation for the HomeServer Assistant deployment assets.

Runs without root and without executing any shell: it parses YAML/JSON,
checks shell script conventions, systemd units, the nftables baseline,
the Caddyfile and the autoinstall seed. Intended for CI and for developers on
Windows where `bash -n` is unavailable.

Usage:
    python3 scripts/check-assets.py [--root PATH] [--verbose]

Exit code 0 when every check passes, 1 otherwise. No third-party packages are
required; PyYAML is used for YAML when installed (a warning is emitted when it
is missing and YAML files are only syntax-scanned).
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path
from typing import Iterable

try:  # PyYAML is optional: CI installs it, minimal hosts may not have it.
    import yaml  # type: ignore
except ImportError:  # pragma: no cover - depends on the environment
    yaml = None  # type: ignore

REPO_ROOT = Path(__file__).resolve().parent.parent
SCAN_DIRS = ["infra", "iso", "scripts"]
SCAN_FILES = ["docs/DEPLOYMENT.md", "docs/ISO.md"]

SHELL_FILES = [
    "scripts/install.sh",
    "iso/build-iso.sh",
    "iso/test-vm.sh",
    "infra/systemd/hs-power-policy.sh",
    "iso/rootfs-overlay/usr/local/bin/hs-kiosk-start",
    "iso/rootfs-overlay/usr/local/sbin/hs-firstboot.sh",
    "iso/rootfs-overlay/usr/local/sbin/hs-doctor.sh",
]

# Explicit exceptions to the "set -euo pipefail" convention.
SET_EUO_EXCEPTIONS = {
    "iso/rootfs-overlay/usr/local/sbin/hs-doctor.sh": "read-only diagnostic: must run every check",
}

SECRET_PATTERNS = [
    re.compile(r"-----BEGIN [A-Z ]*PRIVATE KEY-----"),
    re.compile(r"\bAKIA[0-9A-Z]{16}\b"),  # AWS access key id
    re.compile(r"\bghp_[A-Za-z0-9]{20,}\b"),  # GitHub token
    re.compile(r"\bsk-[A-Za-z0-9]{20,}\b"),  # generic API key shape
    re.compile(r"(?i)\b(password|passwd|token|secret)\s*[:=]\s*['\"]?[^'\"\s$<{]{8,}"),
]

# Values that are explicitly placeholders or documentation examples.
SECRET_ALLOW = re.compile(
    r"(?i)(__[A-Z0-9_]+__|replace|example|placeholder|changeme|dummy|"
    r"<[^>]+>|\$\{|hash|openssl|keyfile|privatekey|password-file|"
    r"\.example|generated|none|false|true)"
)

WINDOWS_PATH = re.compile(r"[A-Za-z]:\\\\|[A-Za-z]:/Users/|\\\\Users\\\\")

results: list[tuple[str, bool, str]] = []


def check(name: str, ok: bool, detail: str = "") -> None:
    results.append((name, ok, detail))
    marker = "PASS" if ok else "FAIL"
    print(f"{marker}  {name}{(': ' + detail) if detail and not ok else ''}")


def warn(name: str, detail: str) -> None:
    print(f"WARN  {name}: {detail}")


def iter_scan_files(root: Path) -> Iterable[Path]:
    seen: set[Path] = set()
    for directory in SCAN_DIRS:
        base = root / directory
        if not base.exists():
            continue
        for path in sorted(base.rglob("*")):
            if path.is_file() and path not in seen:
                seen.add(path)
                yield path
    for rel in SCAN_FILES:
        path = root / rel
        if path.is_file() and path not in seen:
            seen.add(path)
            yield path


def rel(path: Path, root: Path) -> str:
    try:
        # POSIX separators keep the exception/requirement tables portable
        # between Linux CI and a Windows development checkout.
        return path.relative_to(root).as_posix()
    except ValueError:
        return path.as_posix()


def check_text_file(path: Path, root: Path) -> None:
    raw = path.read_bytes()
    rel_path = rel(path, root)
    if rel_path == "scripts/check-assets.py":
        # Do not scan the scanner: its own regex literals look like findings.
        return
    if b"\r\n" in raw:
        check(f"{rel_path}: LF line endings", False, "CRLF found (scripts would fail on Linux)")
    if b"\t" in raw and path.suffix == ".sh":
        warn(f"{rel_path}: tabs", "shell scripts should use spaces for indentation")
    try:
        text = raw.decode("utf-8")
    except UnicodeDecodeError:
        check(f"{rel_path}: UTF-8", False, "file is not valid UTF-8")
        return
    if WINDOWS_PATH.search(text):
        check(f"{rel_path}: no absolute Windows paths", False, "found a developer-machine path")
    for pattern in SECRET_PATTERNS:
        for match in pattern.finditer(text):
            line = text[: match.start()].count("\n") + 1
            snippet = text.splitlines()[line - 1].strip()
            if SECRET_ALLOW.search(snippet):
                continue
            check(f"{rel_path}: no committed secrets", False, f"line {line}: {snippet[:80]}")
            return


def check_shell(path: Path, root: Path) -> None:
    rel_path = rel(path, root)
    text = path.read_text(encoding="utf-8", errors="replace")
    lines = text.splitlines()
    if not lines:
        check(f"{rel_path}: non-empty", False)
        return
    first = lines[0].strip()
    check(f"{rel_path}: shebang", first.startswith("#!"), f"first line: {first!r}")
    if not first.startswith("#!/usr/bin/env bash"):
        warn(f"{rel_path}: shebang", "expected '#!/usr/bin/env bash' for portability")
    has_set = "set -euo pipefail" in text
    excused = SET_EUO_EXCEPTIONS.get(rel_path)
    if excused:
        ok = "set -u" in text
        check(f"{rel_path}: set -u pipefail ({excused})", ok)
    else:
        check(f"{rel_path}: set -euo pipefail", has_set)
    # Cheap structural check that catches the most common paste errors without
    # a shell parser: heredoc openers and `EOF` terminators at column 0.
    opens = len(re.findall(r"<<-?\s*'?(\w+)'?", text))
    closes = len(re.findall(r"^EOF$", text, re.MULTILINE))
    if opens != closes:
        warn(f"{rel_path}: heredocs", f"{opens} handler(s) but {closes} EOF terminator(s)")
    # Functions defined but never called are a common copy/paste smell.
    defined = set(re.findall(r"^([a-zA-Z_][a-zA-Z0-9_]*)\(\) \{", text, re.MULTILINE))
    for name in sorted(defined):
        uses = len(re.findall(rf"\b{re.escape(name)}\b", text))
        if uses <= 1:
            warn(f"{rel_path}: unused function", name)


def check_unit(path: Path, root: Path) -> None:
    rel_path = rel(path, root)
    text = path.read_text(encoding="utf-8", errors="replace")
    ok = all(section in text for section in ("[Unit]", "[Service]", "[Install]"))
    check(f"{rel_path}: systemd sections", ok)
    if "[Service]" in text and "ExecStart=" not in text:
        check(f"{rel_path}: ExecStart", False)


def check_yaml(path: Path, root: Path) -> None:
    rel_path = rel(path, root)
    text = path.read_text(encoding="utf-8", errors="replace")
    if yaml is None:
        # Minimal structural sanity when PyYAML is unavailable.
        bad = [ln for ln in text.splitlines() if "\t" in ln]
        check(f"{rel_path}: YAML (pyyaml missing, tab scan)", not bad, "tabs are invalid YAML indentation")
        return
    try:
        parsed = yaml.safe_load(text)
    except Exception as exc:  # noqa: BLE001 - report the parser error verbatim
        check(f"{rel_path}: YAML parses", False, str(exc).replace("\n", " ")[:200])
        return
    check(f"{rel_path}: YAML parses", True)
    if rel_path.endswith("autoinstall/user-data"):
        check_autoinstall(parsed)
    if rel_path.endswith("autoinstall/meta-data"):
        check(
            "autoinstall/meta-data: instance-id",
            isinstance(parsed, dict) and bool(parsed.get("instance-id")),
        )
        check(
            "autoinstall/meta-data: local-hostname",
            isinstance(parsed, dict) and bool(parsed.get("local-hostname")),
        )
    if "docker-compose" in rel_path:
        check_compose(parsed)


def check_compose(parsed: object) -> None:
    if not isinstance(parsed, dict):
        check("compose: mapping root", False)
        return
    services = parsed.get("services")
    check("compose: services mapping", isinstance(services, dict) and bool(services))
    if isinstance(services, dict):
        for name, service in services.items():
            check(f"compose: service {name} mapping", isinstance(service, dict))
            if isinstance(service, dict):
                check(f"compose: service {name} image pinned", "image" in service)


def check_autoinstall(parsed: object) -> None:
    if not isinstance(parsed, dict):
        check("autoinstall: mapping root", False)
        return
    auto = parsed.get("autoinstall")
    check("autoinstall: top-level section", isinstance(auto, dict))
    if not isinstance(auto, dict):
        return
    check("autoinstall: version 1", auto.get("version") == 1)
    for section in ("identity", "storage", "late-commands", "packages"):
        check(f"autoinstall: {section} present", section in auto)
    identity = auto.get("identity") or {}
    password = str(identity.get("password", ""))
    check(
        "autoinstall: password is a SHA-512 crypt hash (change before shipping)",
        password.startswith("$6$"),
        "generate with: openssl passwd -6",
    )
    ssh = auto.get("ssh") or {}
    check("autoinstall: SSH server disabled by default", ssh.get("install-server") is False)
    late = auto.get("late-commands") or []
    joined = "\n".join(str(item) for item in late) if isinstance(late, list) else ""
    check("autoinstall: late-commands run scripts/install.sh", "scripts/install.sh" in joined)
    check(
        "autoinstall: swap file created",
        "swapfile" in joined,
        "curtin has no swap-file storage type; create it in late-commands",
    )
    storage = auto.get("storage") or {}
    layout = storage.get("layout") or {}
    check("autoinstall: storage layout defined", "name" in layout)


def check_nftables(path: Path, root: Path) -> None:
    rel_path = rel(path, root)
    text = path.read_text(encoding="utf-8", errors="replace")
    required = [
        "table inet homeserver",
        "policy drop",
        'iifname "br-dmz" oifname "br-lan"',
        "DMZ must never reach the administrator portal",
        "table ip homeserver_nat",
        "masquerade",
        "iifname \"br-dmz\" oifname \"eth0\"",
    ]
    for needle in required:
        check(f"{rel_path}: contains {needle!r}", needle in text)
    # The DMZ -> LAN drop must appear before the DMZ -> WAN accept.
    drop_idx = text.find('iifname "br-dmz" oifname "br-lan"')
    accept_idx = text.find('iifname "br-dmz" oifname "eth0"')
    check(
        f"{rel_path}: DMZ->LAN drop ordered before DMZ->WAN accept",
        drop_idx != -1 and accept_idx != -1 and drop_idx < accept_idx,
    )
    check(f"{rel_path}: idempotent reload preamble", "delete table inet homeserver" in text)


def check_caddyfile(path: Path, root: Path) -> None:
    rel_path = rel(path, root)
    text = path.read_text(encoding="utf-8", errors="replace")
    check(f"{rel_path}: braces balanced", text.count("{") == text.count("}"))
    pairs = [
        ("tls internal", "tls internal"),
        ("admin off", "admin off"),
        ("reverse_proxy 127.0.0.1:8080", "127.0.0.1:8080"),
        ("apps.d import", "import /etc/caddy/apps.d/*.caddy"),
        ("admin port placeholder", "{$HS_ADMIN_PORT:8443}"),
        ("X-Forwarded-Proto https", "X-Forwarded-Proto https"),
        ("Content-Security-Policy", "Content-Security-Policy"),
    ]
    for label, needle in pairs:
        check(f"{rel_path}: {label}", needle in text)
    # The kiosk port may only appear in comments; no site may proxy to it.
    proxied_ports = re.findall(r"^\s*reverse_proxy\s+(\S+)", text, re.MULTILINE)
    check(
        f"{rel_path}: kiosk port not proxied",
        all(":3000" not in target and ":8080" in target for target in proxied_ports),
        f"reverse_proxy targets: {proxied_ports}",
    )


def check_nftables_loader(path: Path, root: Path) -> None:
    rel_path = rel(path, root)
    text = path.read_text(encoding="utf-8", errors="replace")
    check(
        f"{rel_path}: includes /etc/nftables.d snippets",
        'include "/etc/nftables.d/*.nft"' in text,
    )
    check(f"{rel_path}: does not define rules itself", "table inet" not in text)


def check_sysusers_tmpfiles(root: Path) -> None:
    sysusers = root / "infra/systemd/sysusers.d/homecore.conf"
    if sysusers.is_file():
        text = sysusers.read_text()
        check("sysusers: hs-vision user", "u hs-vision" in text)
        check("sysusers: kiosk user", "u kiosk" in text)
    tmpfiles = root / "infra/systemd/tmpfiles.d/homecore.conf"
    if tmpfiles.is_file():
        text = tmpfiles.read_text()
        for needle in ("/var/lib/homecore", "/run/homecore", "/etc/caddy/apps.d"):
            check(f"tmpfiles: {needle}", needle in text)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", default=str(REPO_ROOT), help="repository root")
    parser.add_argument("--verbose", action="store_true")
    args = parser.parse_args()
    root = Path(args.root).resolve()

    print(f"checking deployment assets under {root}")
    print(f"PyYAML: {'available' if yaml is not None else 'MISSING (YAML only tab-scanned)'}\n")

    shell_paths: list[Path] = []
    yaml_extra = {"iso/autoinstall/user-data", "iso/autoinstall/meta-data"}
    for path in iter_scan_files(root):
        suffix = path.suffix.lower()
        rel_path = rel(path, root)
        if suffix in {".yaml", ".yml"} or rel_path in yaml_extra:
            check_yaml(path, root)
        elif suffix == ".json":
            try:
                json.loads(path.read_text(encoding="utf-8"))
                check(f"{rel(path, root)}: JSON parses", True)
            except json.JSONDecodeError as exc:
                check(f"{rel(path, root)}: JSON parses", False, str(exc))
        elif suffix == ".sh" or path.name in {"hs-kiosk-start", "hs-firstboot.sh", "hs-doctor.sh"}:
            shell_paths.append(path)
        if path.suffix == ".service":
            check_unit(path, root)
        if path.suffix in {".md"}:
            continue
        check_text_file(path, root)

    for path in sorted(set(shell_paths)):
        check_shell(path, root)

    for candidate in [
        "infra/firewall/nftables.conf",
        "iso/rootfs-overlay/etc/nftables.conf",
        "infra/services/caddy/Caddyfile",
    ]:
        path = root / candidate
        if not path.is_file():
            continue
        if candidate == "infra/firewall/nftables.conf":
            check_nftables(path, root)
        elif candidate == "iso/rootfs-overlay/etc/nftables.conf":
            check_nftables_loader(path, root)
        else:
            check_caddyfile(path, root)

    check_sysusers_tmpfiles(root)

    # Executable-bit intent: git tracks it; Windows checkouts cannot set it.
    executable_hint = root / "scripts/install.sh"
    if executable_hint.is_file() and not (executable_hint.stat().st_mode & 0o111):
        warn(
            "scripts/install.sh is not executable in this checkout",
            "Windows checkouts lose the mode; install.sh/build-iso.sh chmod explicitly",
        )

    failed = [item for item in results if not item[1]]
    print(f"\n{len(results) - len(failed)} checks passed, {len(failed)} failed")
    if failed:
        print("\nfailed checks:")
        for name, _, detail in failed:
            print(f"  - {name}{(': ' + detail) if detail else ''}")
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
