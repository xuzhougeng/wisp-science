# Pet

Wisp Science supports one user-selected Codex-compatible v2 animated pet. Pet support is off by default and the app does not scan a default directory.

## Enable a pet

1. Open **Settings > Pet**.
2. Choose the pet installation folder. The folder must contain `pet.json` and the spritesheet named by `spritesheetPath`.
3. Turn on **Show pet** and save.

The pet folder must use `spriteVersionNumber: 2` and contain a `1536x2288` PNG or WebP atlas arranged as 8 columns by 11 rows of `192x208` cells. Replacing the configured folder, or replacing its compatible files and saving the setting again, changes the active pet. Turning **Show pet** off stops loading and displaying it.

The app uses the standard animation rows for idle, directional walking, waving, jumping, failure, waiting for user input, active work, review, and 16-direction pointer tracking. Clicking an idle pet makes it wave. Reduced-motion system preferences disable roaming and animated playback.

## 配置宠物

Wisp Science 只加载一个由用户明确选择的 Codex v2 宠物，默认关闭，也不会自动扫描任何目录。

打开 **设置 > 宠物**，选择包含 `pet.json` 和精灵表的安装目录，开启 **显示宠物** 后保存。目录必须使用 `spriteVersionNumber: 2`，精灵表必须是 `1536x2288` 的 PNG 或 WebP 文件。更换配置目录即可更换宠物；关闭开关后，应用不会再加载或显示该目录中的宠物。
