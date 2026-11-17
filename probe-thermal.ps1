$ErrorActionPreference = 'Continue'
Write-Output '--- elevated? ---'
([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)

Write-Output '--- counter sets matching thermal/temperature ---'
try {
    Get-Counter -ListSet * -ErrorAction Stop |
        Where-Object { $_.CounterSetName -match 'therm|temper' } |
        ForEach-Object {
            $_.CounterSetName
            $_.Paths | Select-Object -First 12 | ForEach-Object { "    $_" }
        }
} catch {
    Write-Output ("LISTSET ERR: " + $_.Exception.Message)
}

Write-Output '--- Thermal Zone Information(*)\Temperature ---'
try {
    Get-Counter '\Thermal Zone Information(*)\Temperature' -ErrorAction Stop |
        Select-Object -ExpandProperty CounterSamples |
        Select-Object InstanceName, CookedValue, Status |
        Format-Table -AutoSize | Out-String -Width 200
} catch {
    Write-Output ("COUNTER ERR: " + $_.Exception.Message)
}

Write-Output '--- WMI MSAcpi_ThermalZoneTemperature ---'
try {
    Get-CimInstance -Namespace root/WMI -ClassName MSAcpi_ThermalZoneTemperature -ErrorAction Stop |
        Select-Object InstanceName, CurrentTemperature |
        Format-Table -AutoSize | Out-String -Width 200
} catch {
    Write-Output ("WMI ERR: " + $_.Exception.Message)
}

Write-Output '--- Win32_PerfRawData counters for thermal ---'
try {
    Get-CimInstance -ClassName Win32_PerfRawData_Counters_ThermalZoneInformation -ErrorAction Stop |
        Select-Object Name, Temperature, HighPrecisionTemperature |
        Format-Table -AutoSize | Out-String -Width 200
} catch {
    Write-Output ("PERFRAW ERR: " + $_.Exception.Message)
}
