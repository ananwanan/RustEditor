# Requires an interactive Windows desktop. Run a built binary, never cargo run.
# This samples process memory and window discovery; it does not measure presentation.
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$Binary,
    [Parameter(Mandatory = $true)]
    [string]$Output,
    [Parameter(Mandatory = $true)]
    [ValidateRange(50, 500)]
    [int]$DisplayScalePercent,
    [ValidateSet('release', 'debug')]
    [string]$BuildMode = 'release',
    [ValidateRange(1, 100)]
    [int]$Samples = 5,
    [ValidateRange(1, 60)]
    [int]$SettleSeconds = 5,
    [ValidateRange(1, 100)]
    [int]$MemorySamples = 10,
    [ValidateRange(10, 1000)]
    [int]$MemoryIntervalMilliseconds = 100,
    [ValidateRange(1, 120)]
    [int]$TimeoutSeconds = 30,
    [string]$EnvironmentNote = ''
)

$ErrorActionPreference = 'Stop'
if ($env:OS -ne 'Windows_NT') {
    throw 'This harness requires Windows and an interactive desktop.'
}
$binaryPath = (Resolve-Path -LiteralPath $Binary).Path
if (-not (Test-Path -LiteralPath $binaryPath -PathType Leaf) -or
    [System.IO.Path]::GetExtension($binaryPath) -ne '.exe') {
    throw 'Binary must identify a built Windows .exe.'
}
$outputPath = [System.IO.Path]::GetFullPath($Output)
if (Test-Path -LiteralPath $outputPath) {
    throw 'Output already exists; choose a new path to preserve raw measurements.'
}
$repository = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../..'))
$outputDirectory = [System.IO.Path]::GetDirectoryName($outputPath)
[System.IO.Directory]::CreateDirectory($outputDirectory) | Out-Null

function Get-MachineDescription {
    $os = Get-CimInstance Win32_OperatingSystem
    $processors = @(Get-CimInstance Win32_Processor)
    $system = Get-CimInstance Win32_ComputerSystem
    $graphics = @(Get-CimInstance Win32_VideoController | ForEach-Object {
        [ordered]@{
            name = $_.Name
            driver_version = $_.DriverVersion
            reported_width_pixels = $_.CurrentHorizontalResolution
            reported_height_pixels = $_.CurrentVerticalResolution
            reported_refresh_hz = $_.CurrentRefreshRate
        }
    })
    $disks = @()
    $diskQueryError = $null
    try {
        $disks = @(Get-PhysicalDisk | ForEach-Object {
            [ordered]@{
                model = $_.FriendlyName
                media_type = [string]$_.MediaType
                bus_type = [string]$_.BusType
                size_bytes = $_.Size
            }
        })
    } catch {
        # Do not include machine identifiers or arbitrary exception details.
        $diskQueryError = 'Get-PhysicalDisk unavailable; record SSD status manually.'
    }
    [ordered]@{
        os = [ordered]@{
            caption = $os.Caption
            version = $os.Version
            build = $os.BuildNumber
            architecture = $os.OSArchitecture
        }
        cpus = @($processors | ForEach-Object {
            [ordered]@{
                name = $_.Name
                physical_cores = $_.NumberOfCores
                logical_processors = $_.NumberOfLogicalProcessors
            }
        })
        physical_memory_bytes = $system.TotalPhysicalMemory
        graphics = $graphics
        disks = $disks
        disk_query_note = $diskQueryError
        display_scale_percent_operator_reported = $DisplayScalePercent
        environment_note = $EnvironmentNote
    }
}

function Invoke-GuiSample([int]$Index, [bool]$Priming) {
    $process = New-Object System.Diagnostics.Process
    $process.StartInfo.FileName = $binaryPath
    $process.StartInfo.Arguments = '--no-plugins --empty'
    $process.StartInfo.UseShellExecute = $false
    $process.StartInfo.WorkingDirectory = [System.IO.Path]::GetDirectoryName($binaryPath)
    $clock = [System.Diagnostics.Stopwatch]::StartNew()
    $startedUtc = [DateTime]::UtcNow.ToString('o')
    $cleanup = 'not started'
    $result = [ordered]@{
        index = $Index
        priming = $Priming
        started_utc = $startedUtc
        process_id = $null
        launch_to_window_observed_ms = $null
        working_set_bytes = @()
        private_bytes = @()
        memory_sample_elapsed_ms = @()
        cleanup = $null
        failure = $null
    }
    try {
        if (-not $process.Start()) { throw 'Process start failed.' }
        $result.process_id = $process.Id
        do {
            $process.Refresh()
            if ($process.HasExited) { throw 'GUI exited before a main window was observed.' }
            if ($process.MainWindowHandle -ne [IntPtr]::Zero) { break }
            if ($clock.Elapsed.TotalSeconds -ge $TimeoutSeconds) {
                throw 'Timed out waiting for the main window.'
            }
            Start-Sleep -Milliseconds 10
        } while ($true)
        $result.launch_to_window_observed_ms = $clock.Elapsed.TotalMilliseconds
        Start-Sleep -Seconds $SettleSeconds
        for ($i = 0; $i -lt $MemorySamples; $i++) {
            $process.Refresh()
            if ($process.HasExited) { throw 'GUI exited during idle memory sampling.' }
            $result.working_set_bytes += $process.WorkingSet64
            $result.private_bytes += $process.PrivateMemorySize64
            $result.memory_sample_elapsed_ms += $clock.Elapsed.TotalMilliseconds
            Start-Sleep -Milliseconds $MemoryIntervalMilliseconds
        }
    } catch {
        $result.failure = $_.Exception.Message
    } finally {
        $clock.Stop()
        try {
            if ($null -ne $result.process_id -and -not $process.HasExited) {
                # This is the untouched scratch process created above, never a discovered user PID.
                if ($process.CloseMainWindow() -and $process.WaitForExit(5000)) {
                    $cleanup = 'normal close'
                } else {
                    $process.Kill()
                    $process.WaitForExit(5000) | Out-Null
                    $cleanup = 'forced after close timeout; sample requires inspection'
                }
            } elseif ($null -ne $result.process_id) {
                $cleanup = 'already exited'
            }
        } catch {
            $cleanup = 'cleanup failed; process may still be running'
            $detail = "Cleanup: $($_.Exception.Message)"
            $result.failure = if ($null -eq $result.failure) { $detail } else { "$($result.failure); $detail" }
        }
        try {
            $process.Dispose()
        } catch {
            $cleanup = "$cleanup; process disposal failed"
            $detail = "Dispose: $($_.Exception.Message)"
            $result.failure = if ($null -eq $result.failure) { $detail } else { "$($result.failure); $detail" }
        }
        $result.cleanup = $cleanup
    }
    return $result
}

function Get-NearestRank([double[]]$Values, [double]$Percentile) {
    if ($Values.Count -eq 0) { return $null }
    $sorted = @($Values | Sort-Object)
    return $sorted[[int][Math]::Ceiling($Percentile * $sorted.Count) - 1]
}

$revision = (& git -C $repository rev-parse HEAD 2>$null)
if ($LASTEXITCODE -ne 0) { throw 'Cannot record the repository revision.' }
$dirty = @(& git -C $repository status --porcelain --untracked-files=no)
if ($LASTEXITCODE -ne 0) { throw 'Cannot record the tracked working-tree state.' }
$toolchain = @(& rustc --version --verbose)
if ($LASTEXITCODE -ne 0) { throw 'Cannot record the Rust toolchain.' }
$report = [ordered]@{
    schema_version = 1
    recorded_utc = [DateTime]::UtcNow.ToString('o')
    revision = [string]$revision
    tracked_worktree_dirty = $dirty.Count -ne 0
    binary_name = [System.IO.Path]::GetFileName($binaryPath)
    binary_sha256 = (Get-FileHash -LiteralPath $binaryPath -Algorithm SHA256).Hash.ToLowerInvariant()
    build_mode_operator_reported = $BuildMode
    rustc = $toolchain
    command_arguments = @('--no-plugins', '--empty')
    cache_condition = 'warm: one untimed GUI priming launch; OS cache not cleared'
    machine = Get-MachineDescription
    configuration = [ordered]@{
        measured_launches = $Samples
        settle_seconds_after_window = $SettleSeconds
        memory_samples_per_launch = $MemorySamples
        memory_interval_ms = $MemoryIntervalMilliseconds
        window_poll_interval_ms = 10
        launch_timeout_seconds = $TimeoutSeconds
    }
    limitations = @(
        'Window observation is a process/window diagnostic, not editable-state or frame-presentation startup.',
        'Working set is Windows resident process memory, not total system/GPU memory.',
        'Build mode and display scale are operator-reported; verify them before publishing.',
        'No input is injected; input-to-present latency and cold startup remain Pending.'
    )
    priming = $null
    samples = @()
    summary = $null
    run_failure = $null
    acceptance = [ordered]@{
        warm_editable_startup = 'Pending: correlate launch with editable presented frame.'
        cold_editable_startup = 'Pending: controlled reboot/cache condition and presentation evidence required.'
        input_to_present_latency = 'Pending: correlated input and displayed-frame evidence required.'
        idle_memory = 'Pending'
    }
}

# Reserve the report exclusively before starting any measured process. The earlier
# existence check is only diagnostic; CreateNew also closes concurrent-writer races.
$outputStream = [System.IO.File]::Open(
    $outputPath,
    [System.IO.FileMode]::CreateNew,
    [System.IO.FileAccess]::Write,
    [System.IO.FileShare]::None
)
try {
    Write-Host 'Do not interact with the measured editor windows. Priming the warm cache...'
    $report.priming = Invoke-GuiSample 0 $true
    if ($null -ne $report.priming.failure) { throw $report.priming.failure }
    if ($report.priming.cleanup -ne 'normal close') { throw 'Priming GUI did not close normally.' }
    for ($sampleIndex = 1; $sampleIndex -le $Samples; $sampleIndex++) {
        Write-Host "Measuring warm launch $sampleIndex of $Samples..."
        $sample = Invoke-GuiSample $sampleIndex $false
        $report.samples += $sample
        if ($null -ne $sample.failure) { throw $sample.failure }
        if ($sample.cleanup -ne 'normal close') { throw 'Measured GUI did not close normally.' }
    }
    $workingSets = @($report.samples | ForEach-Object { $_.working_set_bytes })
    $privateBytes = @($report.samples | ForEach-Object { $_.private_bytes })
    $windows = @($report.samples | ForEach-Object { $_.launch_to_window_observed_ms })
    $peakWorkingSet = ($workingSets | Measure-Object -Maximum).Maximum
    $report.summary = [ordered]@{
        launch_to_window_observed_ms_p50 = Get-NearestRank $windows 0.50
        launch_to_window_observed_ms_p95 = Get-NearestRank $windows 0.95
        idle_working_set_mib_p50 = (Get-NearestRank $workingSets 0.50) / 1MB
        idle_working_set_mib_max = $peakWorkingSet / 1MB
        idle_private_mib_p50 = (Get-NearestRank $privateBytes 0.50) / 1MB
        idle_private_mib_max = ($privateBytes | Measure-Object -Maximum).Maximum / 1MB
    }
    $report.acceptance.idle_memory = if ($peakWorkingSet -lt 150MB) {
        'Measured below 150 MiB across sampled idle working sets; hardware/build conditions still apply.'
    } else {
        'Measured at or above the 150 MiB idle working-set budget.'
    }
} catch {
    $report.run_failure = $_.Exception.Message
    throw
} finally {
    try {
        $json = $report | ConvertTo-Json -Depth 12
        $encoding = New-Object System.Text.UTF8Encoding($false)
        $bytes = $encoding.GetBytes($json)
        $outputStream.Write($bytes, 0, $bytes.Length)
        $outputStream.Flush($true)
    } finally {
        $outputStream.Dispose()
    }
    Write-Host "Raw report written to $outputPath"
}
