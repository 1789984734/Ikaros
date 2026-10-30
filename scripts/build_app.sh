#!/usr/bin/env bash
set -e

# 获取项目根目录
ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
DIST_DIR="$ROOT_DIR/dist"
APP_NAME="Ikaros.app"
APP_DIR="$DIST_DIR/$APP_NAME"
CONTENTS_DIR="$APP_DIR/Contents"
MACOS_DIR="$CONTENTS_DIR/MacOS"
RESOURCES_DIR="$CONTENTS_DIR/Resources"

echo "=================================================="
echo "🚀 开始打包 macOS 应用程序: $APP_NAME"
echo "=================================================="

# 1. 编译 Release 优化版本
echo "📦 正在编译 Release 版本 (ikaros)..."
cargo build --release --bin ikaros --manifest-path "$ROOT_DIR/Cargo.toml"

# 2. 主图或生成脚本更新时，重新生成应用图标
if [ ! -f "$ROOT_DIR/assets/AppIcon.icns" ] \
    || [ "$ROOT_DIR/assets/icon_1024.png" -nt "$ROOT_DIR/assets/AppIcon.icns" ] \
    || [ "$ROOT_DIR/scripts/generate_icon.py" -nt "$ROOT_DIR/assets/AppIcon.icns" ]; then
    echo "🎨 生成应用程序图标..."
    python3 "$ROOT_DIR/scripts/generate_icon.py"
fi

# 3. 创建 .app 目录结构
echo "📁 创建应用程序目录结构..."
rm -rf "$APP_DIR"
mkdir -p "$MACOS_DIR"
mkdir -p "$RESOURCES_DIR"

# 4. 拷贝可执行文件与资源
echo "📥 拷贝二进制与图标..."
cp "$ROOT_DIR/target/release/ikaros" "$MACOS_DIR/ikaros"
chmod +x "$MACOS_DIR/ikaros"

cp "$ROOT_DIR/assets/AppIcon.icns" "$RESOURCES_DIR/AppIcon.icns"

# 5. 生成 Info.plist
echo "📝 生成 Info.plist 元数据..."
cat << 'EOF' > "$CONTENTS_DIR/Info.plist"
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleDevelopmentRegion</key>
    <string>zh_CN</string>
    <key>CFBundleExecutable</key>
    <string>ikaros</string>
    <key>CFBundleIconFile</key>
    <string>AppIcon</string>
    <key>CFBundleIdentifier</key>
    <string>dev.ikaros.app</string>
    <key>CFBundleInfoDictionaryVersion</key>
    <string>6.0</string>
    <key>CFBundleName</key>
    <string>Ikaros</string>
    <key>CFBundleDisplayName</key>
    <string>Ikaros</string>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
    <key>CFBundleShortVersionString</key>
    <string>0.1.0</string>
    <key>CFBundleVersion</key>
    <string>1</string>
    <key>LSMinimumSystemVersion</key>
    <string>11.0</string>
    <key>NSHighResolutionCapable</key>
    <true/>
    <key>NSSupportsAutomaticGraphicsSwitching</key>
    <true/>
    <key>NSPrincipalClass</key>
    <string>NSApplication</string>
</dict>
</plist>
EOF

# 6. 生成可双击快速启动的终端脚本（作为轻量替代方案）
echo "🔗 生成辅助启动脚本..."
cat << 'EOF' > "$DIST_DIR/启动Ikaros.command"
#!/usr/bin/env bash
DIR="$(cd "$(dirname "$0")" && pwd)"
open "$DIR/Ikaros.app"
EOF
chmod +x "$DIST_DIR/启动Ikaros.command"

# 7. 拷贝安装至 /Applications (本地应用程序)
echo "✨ 正在安装到 /Applications (本地应用程序)..."
if [ -w "/Applications" ]; then
    rm -rf "/Applications/$APP_NAME"
    cp -R "$APP_DIR" "/Applications/$APP_NAME"
    echo "🎉 成功安装到系统应用程序: /Applications/$APP_NAME"
elif [ -w "$HOME/Applications" ]; then
    mkdir -p "$HOME/Applications"
    rm -rf "$HOME/Applications/$APP_NAME"
    cp -R "$APP_DIR" "$HOME/Applications/$APP_NAME"
    echo "🎉 成功安装到用户应用程序: $HOME/Applications/$APP_NAME"
fi

echo "=================================================="
echo "✅ 打包完成！"
echo "📦 独立应用包: $APP_DIR"
echo "💡 您现在可以："
echo "   1. 在 Launchpad（启动台）或 Spotlight (Cmd+Space) 搜索 'Ikaros' 直接回车打开；"
echo "   2. 在 Finder 中直接双击 /Applications/$APP_NAME 启动；"
echo "   3. 将 $APP_NAME 拖拽固定到 Dock 栏方便一键启动。"
echo "=================================================="
