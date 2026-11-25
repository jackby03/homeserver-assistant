#!/usr/bin/env python3
"""Optional local face-embedding helper for HomeServer Assistant OS.

This script is the *only* Python component of the vision workstream. It is used
exclusively during profile enrollment: the daemon sends a still image (for
example a webcam snapshot) here, receives a normalized 512-dimension embedding
on stdout and stores it encrypted through ``hs_core::state::SecretVault``.
Matching, thresholds and security decisions always happen in Rust
(``hs-profiles``); this helper does not compare faces and never decides who a
person is.

Privacy and network behaviour
-----------------------------
* The script performs no network I/O itself.
* InsightFace can download model weights on first use; this helper refuses to
  start unless the requested model is already present under
  ``$INSIGHTFACE_HOME/models/<model>`` (default ``~/.insightface``). Provision
  the model once during setup/imaging, then run fully offline. Pass
  ``--allow-download`` only on a trusted provisioning machine.
* Images are read into memory and never written to disk by this script.

Usage
-----
    python3 vision/python/face_embed.py --image /tmp/enroll.jpg
    python3 vision/python/face_embed.py --stdin < /tmp/enroll.jpg

Output (stdout, one JSON object, no logging):

    {"embedding": [0.012345, ...], "model": "buffalo_l", "dim": 512}

Errors are reported on stderr as ``{"error": "..."}`` with exit codes:

    0  success
    2  invalid input (unreadable/undecodable image)
    3  missing optional dependency or model files
    4  embedding extraction failed (for example no face detected)
"""

from __future__ import annotations

import argparse
import json
import os
import sys
from pathlib import Path

DEFAULT_MODEL = "buffalo_l"
DEFAULT_DET_SIZE = 640
EXIT_INVALID_INPUT = 2
EXIT_MISSING_DEPENDENCY = 3
EXIT_EMBEDDING_FAILED = 4


def parse_args(argv: "list[str]") -> argparse.Namespace:
    """Parse command-line arguments."""
    parser = argparse.ArgumentParser(
        prog="face_embed.py",
        description="Produce a local face embedding with InsightFace (no matching, no cloud).",
    )
    source = parser.add_mutually_exclusive_group(required=True)
    source.add_argument("--image", type=Path, help="Path to a JPEG/PNG/WebP image.")
    source.add_argument(
        "--stdin",
        action="store_true",
        help="Read the raw image bytes from stdin.",
    )
    parser.add_argument(
        "--model",
        default=DEFAULT_MODEL,
        help=f"InsightFace model pack name (default: {DEFAULT_MODEL}).",
    )
    parser.add_argument(
        "--det-size",
        type=int,
        default=DEFAULT_DET_SIZE,
        help=f"Face detector input size in pixels (default: {DEFAULT_DET_SIZE}).",
    )
    parser.add_argument(
        "--allow-download",
        action="store_true",
        help="Permit InsightFace to download the model pack if it is missing (dev only, needs network).",
    )
    args = parser.parse_args(argv)
    if args.det_size <= 0:
        parser.error("--det-size must be a positive integer")
    return args


def read_image_bytes(args: argparse.Namespace) -> bytes:
    """Read the enrollment image either from a file or from stdin."""
    if args.stdin:
        data = sys.stdin.buffer.read()
    else:
        data = args.image.read_bytes()
    if not data:
        raise ValueError("the input image is empty")
    return data


def import_insightface():
    """Import the optional third-party dependencies with a readable error."""
    try:
        import cv2  # noqa: F401 - imported for its side effect of being validated
        import numpy as np
        from insightface.app import FaceAnalysis
    except ImportError as error:  # pragma: no cover - depends on the host
        raise ImportError(str(error)) from error
    return cv2, np, FaceAnalysis


def insightface_home() -> Path:
    """Directory where InsightFace looks for model packs."""
    configured = os.environ.get("INSIGHTFACE_HOME")
    if configured:
        return Path(configured).expanduser()
    return Path.home() / ".insightface"


def extract_embedding(cv2, np, face_analysis_cls, image_bytes: bytes, args) -> list:
    """Decode the image, detect the largest face and return its normalized embedding."""
    image = cv2.imdecode(np.frombuffer(image_bytes, dtype=np.uint8), cv2.IMREAD_COLOR)
    if image is None:
        raise ValueError("OpenCV could not decode the image bytes")

    application = face_analysis_cls(name=args.model, providers=["CPUExecutionProvider"])
    application.prepare(ctx_id=-1, det_size=(args.det_size, args.det_size))
    faces = application.get(image)
    if not faces:
        raise ValueError("no face was detected in the image")

    def face_area(face) -> float:
        left, top, right, bottom = (float(value) for value in face.bbox[:4])
        return max(0.0, right - left) * max(0.0, bottom - top)

    face = max(faces, key=face_area)
    embedding = getattr(face, "normed_embedding", None)
    if embedding is None:
        raw = getattr(face, "embedding", None)
        if raw is None:
            raise ValueError(
                "the selected model did not produce an embedding; use a recognition model such as buffalo_l"
            )
        norm = float(np.linalg.norm(raw))
        if norm <= 0.0:
            raise ValueError("the model returned a zero-length embedding")
        embedding = raw / norm
    return [round(float(value), 6) for value in embedding]


def fail(message: str, exit_code: int) -> int:
    """Print a machine-readable error on stderr and return the exit code."""
    print(json.dumps({"error": message}), file=sys.stderr)
    return exit_code


def main(argv: "list[str] | None" = None) -> int:
    """CLI entry point."""
    args = parse_args(sys.argv[1:] if argv is None else argv)

    try:
        image_bytes = read_image_bytes(args)
    except (OSError, ValueError) as error:
        return fail(f"cannot read input image: {error}", EXIT_INVALID_INPUT)

    try:
        cv2, np, face_analysis_cls = import_insightface()
    except ImportError as error:
        return fail(
            f"optional dependency missing: {error}. "
            "Install with: pip install -r vision/python/requirements.txt",
            EXIT_MISSING_DEPENDENCY,
        )

    model_directory = insightface_home() / "models" / args.model
    if not model_directory.is_dir() and not args.allow_download:
        return fail(
            f"InsightFace model '{args.model}' not found in {model_directory}. "
            "Provision it offline first (see vision/README.md) or pass --allow-download.",
            EXIT_MISSING_DEPENDENCY,
        )

    try:
        embedding = extract_embedding(cv2, np, face_analysis_cls, image_bytes, args)
    except Exception as error:  # noqa: BLE001 - third-party code raises generic errors
        return fail(f"face embedding failed: {error}", EXIT_EMBEDDING_FAILED)

    print(
        json.dumps(
            {"embedding": embedding, "model": args.model, "dim": len(embedding)},
            separators=(",", ":"),
        )
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
