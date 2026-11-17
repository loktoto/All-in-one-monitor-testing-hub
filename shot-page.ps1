param(
    [string]$Page = 'overview',
    [int]$WaitSeconds = 15,
    [string]$OutFile = 'shot.png'
)
Get-Process hardware-hub -ErrorAction SilentlyContinue | Stop-Process -Force
Start-Sleep -Milliseconds 900
$env:HH_START_PAGE = $Page
Start-Process -FilePath '.\target\debug\hardware-hub.exe'
Start-Sleep -Seconds $WaitSeconds
& powershell -NoProfile -ExecutionPolicy Bypass -File .\capture-window.ps1 -OutFile $OutFile -DelayMs 900 -Width 1600 -Height 1000 | Write-Output
