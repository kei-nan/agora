# setup-android.ps1
# Generates the React Native android/ project by extracting the template
# directly from the react-native npm package (bypasses `react-native init`
# and its CocoaPods failure on WSL2).
#
# Usage:
#   powershell -ExecutionPolicy Bypass -File "Z:\home\realize\democracy-chain\mobile\setup-android.ps1"

$ErrorActionPreference = "Stop"

Write-Host ""
Write-Host "=== Agora mobile native project setup ===" -ForegroundColor Cyan
Write-Host ""

$bashScript = @'
#!/usr/bin/env bash
set -e

MOBILE_DIR="/home/realize/democracy-chain/mobile"
WORK_DIR="/tmp/AgoraTemplate"

echo ""
echo "[1/3] Downloading react-native@0.74.0 via npm pack (no CLI init)..."
rm -rf "$WORK_DIR"
mkdir -p "$WORK_DIR"
cd "$WORK_DIR"
npm pack react-native@0.74.0

tar -xf react-native-0.74.0.tgz

echo ""
echo "Contents of package/:"
ls "$WORK_DIR/package/"

TMPL="$WORK_DIR/package/template/android"
if [ ! -d "$TMPL" ]; then
  echo ""
  echo "ERROR: template/android not found in react-native@0.74.0 package."
  exit 1
fi

echo ""
echo "[2/3] Patching app name (HelloWorld -> Agora, com.helloworld -> com.agora)..."
rm -rf /tmp/AgoraAndroid
cp -r "$TMPL" /tmp/AgoraAndroid

# Rename Java package directory helloworld -> agora
find /tmp/AgoraAndroid -type d -name "helloworld" | while read d; do
  parent="$(dirname "$d")"
  mkdir -p "$parent/agora"
  mv "$d"/* "$parent/agora/" 2>/dev/null || true
  rmdir "$d" 2>/dev/null || true
done

# Text substitution in every file (sed errors on binaries are suppressed)
find /tmp/AgoraAndroid -type f | while read f; do
  sed -i 's/HelloWorld/Agora/g; s/helloworld/agora/g; s/com\.helloworld/com.agora/g' "$f" 2>/dev/null || true
done

# Ensure gradlew is executable
chmod +x /tmp/AgoraAndroid/gradlew 2>/dev/null || true

echo ""
echo "[3/3] Copying android/ into mobile directory and running npm install..."
rm -rf "$MOBILE_DIR/android"
cp -r /tmp/AgoraAndroid "$MOBILE_DIR/android"
rm -rf /tmp/AgoraAndroid "$WORK_DIR"

cd "$MOBILE_DIR"
npm install

echo ""
echo "=== Setup complete! ==="
echo ""
echo "Verify:"
ls "$MOBILE_DIR/android/"
'@

Write-Host "Writing setup script into WSL..." -ForegroundColor Yellow
# Strip Windows CRLF -> LF before piping to WSL to avoid \r in bash variables
$bashScript.Replace("`r`n", "`n") | wsl -e bash -c "cat > /tmp/agora_setup.sh && chmod +x /tmp/agora_setup.sh"

Write-Host "Running setup inside WSL (takes ~2 minutes)..." -ForegroundColor Yellow
wsl -e bash /tmp/agora_setup.sh

if ($LASTEXITCODE -ne 0) {
    Write-Host ""
    Write-Host "Setup FAILED. Check the output above for errors." -ForegroundColor Red
    exit 1
}

Write-Host ""
Write-Host "Next steps:" -ForegroundColor Cyan
Write-Host "  1. Open Android Studio" -ForegroundColor White
Write-Host "  2. File -> Open -> \\wsl`$\Ubuntu\home\realize\democracy-chain\mobile\android" -ForegroundColor White
Write-Host "  3. Wait for Gradle sync, then Run -> Run 'app'" -ForegroundColor White
Write-Host ""
