# macOS app icon

`Assets.car` and `AppIcon.icns` are compiled from `../AppIcon.icon` (an Icon Composer bundle built from the glyph layers in `packaging/icons/layers/`). They're committed so building the app doesn't need Xcode 26.

After changing the icon, recompile on a Mac with Xcode 26 or newer:

```sh
xcrun actool packaging/macos/AppIcon.icon --compile packaging/macos/AppIcon \
  --platform macosx --minimum-deployment-target 11.0 --app-icon AppIcon \
  --output-partial-info-plist /tmp/AppIcon-partial.plist
```
