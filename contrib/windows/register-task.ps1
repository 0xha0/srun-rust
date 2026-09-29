# Registers a Task Scheduler job that runs "srun daemon" at logon and
# restarts it if it exits. Run from an elevated PowerShell:
#   .\register-task.ps1 -Exe C:\tools\srun.exe
# Remove with: Unregister-ScheduledTask -TaskName srun-daemon -Confirm:$false
param(
    [string]$Exe = "$PSScriptRoot\srun.exe",
    [string]$Config = "$env:APPDATA\srun\config.json"
)
$action = New-ScheduledTaskAction -Execute $Exe -Argument "-c `"$Config`" daemon"
$trigger = New-ScheduledTaskTrigger -AtLogOn
$settings = New-ScheduledTaskSettingsSet -RestartCount 999 -RestartInterval (New-TimeSpan -Minutes 1) `
    -ExecutionTimeLimit (New-TimeSpan -Days 3650) -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries
Register-ScheduledTask -TaskName "srun-daemon" -Action $action -Trigger $trigger -Settings $settings -Force
Start-ScheduledTask -TaskName "srun-daemon"
Write-Host "registered and started task srun-daemon"
