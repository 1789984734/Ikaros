#!/usr/bin/env bash
set -e

# 获取项目根目录
ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
DIST_DIR="$ROOT_DIR/dist"
CACHE_DIR="$DIST_DIR/.cache"
VERSION="0.1.0"
PACKAGE_NAME="Ikaros-v${VERSION}-windows-x64"
OUTPUT_DIR="$DIST_DIR/$PACKAGE_NAME"
TOOLS_DIR="$OUTPUT_DIR/tools/scrcpy"
ZIP_FILE="$DIST_DIR/${PACKAGE_NAME}.zip"

# 官方发布的 Windows 平台便携包 (含完整匹配的 adb.exe 与 scrcpy.exe 及运行时库)
SCRCPY_VERSION="v5.0.1"
SCRCPY_ZIP="scrcpy-win64-${SCRCPY_VERSION}.zip"
SCRCPY_URL="https://github.com/Genymobile/scrcpy/releases/download/${SCRCPY_VERSION}/${SCRCPY_ZIP}"

echo "=================================================="
echo "🚀 开始打包 Windows 便携绿色版: $PACKAGE_NAME"
echo "=================================================="

# 1. 编译 Release 优化版本 (通过 cargo-zigbuild 进行交叉编译)
echo "📦 正在交叉编译 Windows Release 版本 (ikaros.exe)..."
cargo zigbuild --release --target x86_64-pc-windows-gnu --bin ikaros --manifest-path "$ROOT_DIR/Cargo.toml"

# 2. 准备输出目录结构
echo "📁 准备便携包目录结构..."
rm -rf "$OUTPUT_DIR"
mkdir -p "$OUTPUT_DIR"
mkdir -p "$TOOLS_DIR"
mkdir -p "$CACHE_DIR"

# 3. 拷贝主程序
echo "📥 拷贝主程序 ikaros.exe..."
cp "$ROOT_DIR/target/x86_64-pc-windows-gnu/release/ikaros.exe" "$OUTPUT_DIR/ikaros.exe"

# 4. 获取并解压官方配套的 Windows scrcpy + adb 工具包
if [ ! -f "$CACHE_DIR/$SCRCPY_ZIP" ]; then
    echo "🌐 下载官方 Windows 配套工具包 (scrcpy + adb)..."
    curl -fSL "$SCRCPY_URL" -o "$CACHE_DIR/$SCRCPY_ZIP"
else
    echo "⚡ 使用已缓存的官方配套工具包: $SCRCPY_ZIP"
fi

echo "📦 释放内置工具链到 $TOOLS_DIR ..."
TMP_EXTRACT="$DIST_DIR/.tmp_scrcpy"
rm -rf "$TMP_EXTRACT"
mkdir -p "$TMP_EXTRACT"
unzip -q "$CACHE_DIR/$SCRCPY_ZIP" -d "$TMP_EXTRACT"

# 将解压后的子目录内容移动到 tools/scrcpy
EXTRACTED_SUBDIR=$(find "$TMP_EXTRACT" -mindepth 1 -maxdepth 1 -type d | head -n 1)
if [ -n "$EXTRACTED_SUBDIR" ]; then
    cp -r "$EXTRACTED_SUBDIR"/* "$TOOLS_DIR/"
else
    cp -r "$TMP_EXTRACT"/* "$TOOLS_DIR/"
fi
rm -rf "$TMP_EXTRACT"

# 5. 生成使用说明文件
echo "📝 生成快速使用说明..."
cat << 'EOF' > "$OUTPUT_DIR/使用说明.txt"
==================================================
           Ikaros Android 管理平台
==================================================

【如何使用】
1. 解压本压缩包到任意非中文路径文件夹中。
2. 双击运行 ikaros.exe 即可打开软件。

【特性说明】
* 开箱即用：本软件包已在 tools/scrcpy 目录下预置了官方
  配套的 adb.exe、scrcpy.exe 及完整的视频解码运行库。
* 无需额外安装 Android Studio 或预先配置环境变量。
* 首次连接手机时，请确保手机已开启「开发者选项」及「USB 调试」。

祝您使用愉快！
==================================================
EOF

# 6. 打包成最终的 ZIP 分发包
echo "🗜️ 压缩生成最终分发包: $ZIP_FILE ..."
rm -f "$ZIP_FILE"
(cd "$DIST_DIR" && zip -q -r "$ZIP_FILE" "$PACKAGE_NAME")

echo "=================================================="
echo "🎉 Windows 便携版打包完成！"
echo "📦 目录产物: $OUTPUT_DIR"
echo "🎁 压缩分发包: $ZIP_FILE"
echo "=================================================="
