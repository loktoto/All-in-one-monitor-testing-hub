for ($i = 0; $i -lt 4; $i++) {
    $rows = Get-CimInstance -ClassName Win32_PerfFormattedData_PerfOS_Processor -ErrorAction Stop |
        Select-Object Name, PercentProcessorTime, PercentPrivilegedTime, PercentInterruptTime
    $total = $rows | Where-Object Name -eq '_Total'
    $cores = ($rows | Where-Object { $_.Name -ne '_Total' } | Sort-Object { [int]$_.Name } |
        ForEach-Object { "$($_.Name)=$($_.PercentProcessorTime)" }) -join ' '
    Write-Output ("total=" + $total.PercentProcessorTime + " | " + $cores)
    Start-Sleep -Seconds 1
}
