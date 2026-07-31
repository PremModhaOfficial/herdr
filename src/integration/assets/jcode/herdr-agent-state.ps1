# managed by herdr; reinstalling the integration replaces this file.
# HERDR_INTEGRATION_ID=jcode
# HERDR_INTEGRATION_VERSION=1
#
# Bridges jcode lifecycle hooks to herdr. jcode invokes this script for
# session_start/turn_start/turn_end/session_end with JCODE_HOOK_EVENT set;
# the script no-ops outside herdr panes.

param()
$event = $env:JCODE_HOOK_EVENT
if ($event -notin @('session_start', 'turn_start', 'turn_end', 'session_end')) { exit 0 }
if ($env:HERDR_ENV -ne '1') { exit 0 }
if (-not $env:HERDR_SOCKET_PATH -or -not $env:HERDR_PANE_ID) { exit 0 }

$source = 'herdr:jcode'
$agent = 'jcode'
$paneId = $env:HERDR_PANE_ID
$socketPath = $env:HERDR_SOCKET_PATH
$sessionId = $env:JCODE_HOOK_SESSION_ID

function Send-Herdr([string]$method, [hashtable]$extra) {
    $params = @{
        pane_id = $paneId
        source  = $source
        agent   = $agent
        seq     = [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
    }
    foreach ($key in $extra.Keys) { $params[$key] = $extra[$key] }
    $request = @{ id = "$source:$([DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds())"; method = $method; params = $params } | ConvertTo-Json -Compress
    try {
        $client = New-Object System.Net.Sockets.Socket([System.Net.Sockets.AddressFamily]::Unix, [System.Net.Sockets.SocketType]::Stream, [System.Net.Sockets.ProtocolType]::Tcp)
        $endpoint = New-Object System.Net.Sockets.UnixDomainSocketEndPoint($socketPath)
        $client.Connect($endpoint)
        $bytes = [System.Text.Encoding]::UTF8.GetBytes($request + "`n")
        [void]$client.Send($bytes)
        $buffer = New-Object byte[] 4096
        try { [void]$client.Receive($buffer) } catch { }
        $client.Close()
    } catch {
    }
}

switch ($event) {
    'session_start' {
        $extra = @{}
        if ($sessionId) { $extra['agent_session_id'] = $sessionId }
        $startSource = $env:JCODE_HOOK_SOURCE
        if ($startSource -in @('create', 'attach')) { $extra['session_start_source'] = 'startup' }
        elseif ($startSource -eq 'resume') { $extra['session_start_source'] = 'resume' }
        if ($extra.Count -gt 0) { Send-Herdr 'pane.report_agent_session' $extra }
        Send-Herdr 'pane.report_agent' @{ state = 'idle' }
    }
    'turn_start' { Send-Herdr 'pane.report_agent' @{ state = 'working' } }
    'turn_end' { Send-Herdr 'pane.report_agent' @{ state = 'idle' } }
    'session_end' {
        Send-Herdr 'pane.report_agent' @{ state = 'idle' }
        Send-Herdr 'pane.release_agent' @{}
    }
}
