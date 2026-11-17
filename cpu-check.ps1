$total = (Get-Counter '\Processor(_Total)\% Processor Time' -SampleInterval 1 -MaxSamples 3).CounterSamples |
    ForEach-Object { [math]::Round($_.CookedValue, 1) }
Write-Output ("total: " + ($total -join ', '))
$cores = (Get-Counter '\Processor(*)\% Processor Time' -SampleInterval 1 -MaxSamples 1).CounterSamples |
    Where-Object { $_.InstanceName -match '^\d+$' } |
    Sort-Object InstanceName |
    ForEach-Object { "$($_.InstanceName)=$([math]::Round($_.CookedValue,0))" }
Write-Output ("cores: " + ($cores -join ' '))
$app = Get-Process hardware-hub -ErrorAction SilentlyContinue | Select-Object -First 1
if ($app) {
    $a = $app.CPU
    Start-Sleep -Seconds 3
    $b = (Get-Process -Id $app.Id).CPU
    Write-Output ("app cpu seconds over 3s: " + [math]::Round($b - $a, 2))
}
