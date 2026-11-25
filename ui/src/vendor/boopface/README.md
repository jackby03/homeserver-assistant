# BoopFace (vendored)

- Upstream: https://github.com/CX-ArtLab/BoopFace
- Version: 0.9.0 (`v0.9.0` tag)
- License: MIT (see `LICENSE`)
- Files: `boopface-lite.js` and `src/` copied from the official source archive.

The kiosk uses the **lite** build (no skins) through `BoopAvatar.tsx`. The
component is loaded lazily as a Web Component so the kiosk still renders if the
avatar bundle fails. To upgrade, replace these files with a newer release and
update this note.
