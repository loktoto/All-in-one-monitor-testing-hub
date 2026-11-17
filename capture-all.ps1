$pages = 'overview','cpu','gpu','memory','storage','thermal','motherboard','network','bluetooth','details','processes','frames','tests','reports'
New-Item -ItemType Directory -Force shots | Out-Null
foreach ($p in $pages) {
    Get-Process hardware-hub -ErrorAction SilentlyContinue | Stop-Process -Force
    Start-Sleep -Milliseconds 900
    $env:HH_START_PAGE = $p
    Start-Process -FilePath '.\target\debug\hardware-hub.exe'
    Start-Sleep -Seconds 5
    & powershell -NoProfile -ExecutionPolicy Bypass -File .\capture-window.ps1 -OutFile "shots\$p.png" -DelayMs 900 -Width 1600 -Height 1000
}
Get-Process hardware-hub -ErrorAction SilentlyContinue | Stop-Process -Force
