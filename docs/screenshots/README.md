# Screenshots

These images document the current development build of HomeServer Assistant OS.
They are captured from the real demo environment, with demo profiles, demo
network metrics and demo applications. No credentials or private user data are
shown.

| File | Description |
|---|---|
| `wall-concept.jpg` | Concept render of the wall-mounted appliance. Not a shipping photo. |
| `kiosk.png` | Kiosk home screen with clock, weather, avatar and voice status. |
| `kiosk-automation.png` | Kiosk automation drawer (music, scenes, network). |
| `admin-apps.png` | Admin application catalog and deployment UI. |

## Regenerating

1. Start the daemon in development mode:
   ```bash
   cargo run -p hs-daemon -- serve --dev --data-dir ./.hs-data --static-dir ui/dist
   ```
2. Open `http://127.0.0.1:3000/kiosk` and `http://127.0.0.1:8080/admin`.
3. Capture at 1600×900 with a desktop browser.

Screenshots are committed as PNG/JPEG assets. Keep them free of secrets,
personal data, real LAN addresses and credentials. The screenshot sources are
not part of the application runtime.
