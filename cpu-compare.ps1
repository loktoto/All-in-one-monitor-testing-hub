$a = (Get-Counter '\Processor(_Total)\% Processor Time' -SampleInterval 1 -MaxSamples 2).CounterSamples |
    ForEach-Object { [math]::Round($_.CookedValue, 1) }
$b = (Get-CimInstance Win32_PerfFormattedData_PerfOS_Processor |
    Where-Object Name -eq '_Total').PercentProcessorTime
$c = (Get-Counter '\Processor Information(_Total)\% Processor Utility' -ErrorAction SilentlyContinue).CounterSamples |
    ForEach-Object { [math]::Round($_.CookedValue, 1) }
Write-Output ("Get-Counter total: " + ($a -join ', '))
Write-Output ("WMI formatted total: " + $b)
Write-Output ("Processor Utility: " + ($c -join ', '))

$snap1 = Get-Process | Select-Object Id, ProcessName, CPU
Start-Sleep -Seconds 3
$snap2 = Get-Process | Select-Object Id, ProcessName, CPU
$deltas = foreach ($p in $snap2) {
    $old = $snap1 | Where-Object Id -eq $p.Id
    $old = $old | Select-Object -First 1
    if ($old -and $p.CPU -ne $null -and $old.CPU -ne $null) {
        [pscustomobject]@{ Name = $p.ProcessName; Delta = [math]::Round($p.CPU - $old.CPU, 2) }
    }
}
$deltas | Sort-Object Delta -Descending | Select-Object -First 8 | Format-Table -AutoSize | Out-String -Width 120
Write-Output ("sum of process cpu seconds over 3s (8 cores => 100% = 24s): " + [math]::Round(($deltas | Measure-Object Delta -Sum).Sum, 2))
