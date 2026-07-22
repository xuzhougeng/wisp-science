# Acknowledgement and provenance

Wisp's real-browser bridge is inspired by the **GA Web / TMWebDriver** design
in [lsdefine/GenericAgent](https://github.com/lsdefine/GenericAgent): an agent
connects to a real, persistent Chrome/Chromium profile through a loopback
bridge and a browser extension instead of launching a temporary headless
profile.

GenericAgent is distributed under the MIT License:

> Copyright (c) 2025 lsdefine

The Rust bridge and Manifest V3 extension in Wisp are an independent
implementation. No GenericAgent source files are redistributed here. The
connection model and compatible message shapes were informed by GenericAgent's
public implementation. Wisp is not affiliated with or endorsed by the
GenericAgent project.

Upstream project and license:

- <https://github.com/lsdefine/GenericAgent>
- <https://github.com/lsdefine/GenericAgent/blob/main/LICENSE>
