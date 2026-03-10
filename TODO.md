# TODO

- Add more detailed documentation for the project.
- Implement unit tests for all major functions.
- Refactor the codebase to improve readability and maintainability.
- Rename backend to GstBackend
- FIX: Polling loop in tray_subscription (src/tray.rs:139-174)
- Remove all the buffer pools and use the system native in all systems (macos: OK)
- Change to use BGRA instead of RGBA (seems to be the native for most of the platforms)
- FIX: Last frame commin after stop recording