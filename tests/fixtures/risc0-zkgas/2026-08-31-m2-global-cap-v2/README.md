# Calibration Scope

This fixture was collected with proposal ELF SHA-256
`d7a4aca3769005d30772a6a1d4c47c95f7d6692244a3b017b181935a855e6b35` and image ID
`0xd6ab71c22201c23ef512b706f2e2d720f6da1b559fb76834aa9d4e35276f6e10`, as recorded in
`config.json`.

Those measurements predate the proposal ELF rebuilt by raiko2 #242 and do not use the v0.6.0
release guest. The model identity records what was measured; it is intentionally not a runtime ELF
gate. A release that enables `estimated` with another guest accepts unmeasured quote-price and
timeout drift. Use `evaluated` when the exact cycle count for the running guest is required.
