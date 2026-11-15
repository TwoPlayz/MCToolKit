<#
Builds the Bedrock block registry by asking the real Bedrock Dedicated Server.

  bedrock.ps1 -Server Z:\srv -Work C:\some\folder -Version 1.26.61.1 -Out crates\mctk-registry\data\bedrock\1.26.61.1.tsv

-Server  An extracted Bedrock Dedicated Server (https://www.minecraft.net/en-us/download/server/bedrock).
         Keep its path short: the game's structure files overflow Windows' 260 character limit.
-Work    A folder for the intermediate files (probe lists, results).
-Java    The Java registry of a comparable version, used only to widen the list of ids to try.

How it works (see crates/mctk-gen/src/main.rs): ids come from the executable's strings, the Java
registry, the repo's mapping and the resource pack's blocks.json. The server's own command parser
says which are blocks. Each block is then placed in a cell packed in stone and the saved world is
read back for its real states; every candidate value of every state is placed too, and what the
game refuses stays stone. Nothing here is copied from another project's tables.
#>
param(
  [Parameter(Mandatory)][string]$Server,
  [Parameter(Mandatory)][string]$Work,
  [Parameter(Mandatory)][string]$Version,
  [Parameter(Mandatory)][string]$Out,
  [string]$Java = "crates\mctk-registry\data\java\26.3.tsv",
  [string]$Gen = "target\release\mctk-gen.exe",
  [int]$Port = 19300
)
$ErrorActionPreference = "Stop"
New-Item -ItemType Directory -Force $Work | Out-Null
$stamp = Get-Date -Format "HHmmss"

# Starts the server on a world, runs the commands in $CmdFile and saves.
function Run-Server($level, $cmds, $startup = 30, $end = 8) {
  $props = Get-Content "$Server\server.properties" -Raw
  foreach ($kv in @{ "level-name" = $level; "server-port" = "$Port"; "server-portv6" = "$($Port + 1)"; "online-mode" = "false"; "allow-list" = "false";
                     "allow-cheats" = "true"; "content-log-console-output-enabled" = "true"; "emit-server-telemetry" = "false" }.GetEnumerator()) {
    if ($props -match "(?m)^$($kv.Key)=.*$") { $props = $props -replace "(?m)^$($kv.Key)=.*$", "$($kv.Key)=$($kv.Value)" } else { $props += "`n$($kv.Key)=$($kv.Value)" }
  }
  Set-Content "$Server\server.properties" $props -Encoding ascii
  $psi = New-Object System.Diagnostics.ProcessStartInfo
  $psi.FileName = "$Server\bedrock_server.exe"; $psi.WorkingDirectory = $Server
  $psi.RedirectStandardInput = $true; $psi.RedirectStandardOutput = $true; $psi.UseShellExecute = $false
  $p = [System.Diagnostics.Process]::Start($psi)
  $task = $p.StandardOutput.ReadToEndAsync()
  function Send($c) { $p.StandardInput.WriteLine($c); $p.StandardInput.Flush() }
  Start-Sleep $startup
  Send ""; Start-Sleep 1   # absorbs a UTF-8 byte order mark
  foreach ($c in $cmds) {
    if ($c.StartsWith("#sleep ")) { Start-Sleep ([int]$c.Substring(7)) } else { Send $c; Start-Sleep -Milliseconds 800 }
  }
  Start-Sleep $end
  Send "save hold"; Start-Sleep 3; Send "save query"; Start-Sleep 2; Send "save resume"; Start-Sleep 1; Send "stop"
  if (-not $p.WaitForExit(90000)) { throw "server did not exit" }
  $task.Result
}

# Plans a probe list into a fresh world, runs it (execute) or just loads it (parse only).
function Probe($name, $probes, [switch]$Execute) {
  $level = "$name$stamp"
  $world = "$Server\worlds\$level"
  New-Item -ItemType Directory -Force $world | Out-Null
  $plan = & $Gen plan $probes $world
  Write-Host $plan
  $cmds = @()
  if ($Execute) {
    $area = ($plan -split "tickingarea add ")[1]
    $n = [int](($plan -split "functions \(p0\.\.p")[1] -split "\)")[0]
    $cmds = @("gamerule functioncommandlimit 10000", "tickingarea add $area", "#sleep 30")
    for ($i = 0; $i -le $n; $i++) { $cmds += "function p$i"; $cmds += "#sleep 25" }
  } else { $cmds = @("#sleep 1") }
  $log = Run-Server $level $cmds
  [pscustomobject]@{ World = $world; Log = $log; Plan = $plan }
}

# 1. Candidate ids, and which of them the server's parser accepts as blocks.
& $Gen strings "$Server\bedrock_server.exe" "$Work\exe_ids.txt"
& $Gen mapping-names "$Work\mapping_names.txt"
$packKeys = Get-Content "$Server\resource_packs\vanilla\blocks.json" | Select-String '^  "([a-z0-9_]+)": \{' | ForEach-Object { "minecraft:" + $_.Matches[0].Groups[1].Value }
$javaIds = Get-Content $Java | Where-Object { $_ -like 'B *' } | ForEach-Object { ($_ -split "`t")[0].Substring(2) }
$cand = @(Get-Content "$Work\exe_ids.txt") + @(Get-Content "$Work\mapping_names.txt") + $packKeys + $javaIds | Sort-Object -Unique
Set-Content "$Work\candidates.tsv" $cand -Encoding ascii
$r = Probe "parse" "$Work\candidates.tsv"
$bad = @{}
$r.Log -split "`n" | Select-String "Error on line (\d+)" | ForEach-Object { $bad[[int]$_.Matches[0].Groups[1].Value] = $true }
$fills = (Get-Content "$($r.World)\behavior_packs\probe\functions\p0.mcfunction" | Select-String "^fill").Count
$valid = @(); for ($i = 0; $i -lt $cand.Count; $i++) { if (-not $bad.ContainsKey($fills + 1 + $i)) { $valid += $cand[$i] } }
Set-Content "$Work\probes1.tsv" $valid -Encoding ascii
Write-Host "$($valid.Count) block ids of $($cand.Count) candidates"

# 2. Place every block; read back its real default state.
$r = Probe "ph1" "$Work\probes1.tsv" -Execute
& $Gen collect $r.World "$Work\probes1.tsv" "$Work\results1.tsv"

# 3. Try every candidate value of every state; the game refuses what it does not have.
& $Gen probes2 "$Work\probes1.tsv" "$Work\results1.tsv" "$Work\probes2.tsv"
$r = Probe "ph2" "$Work\probes2.tsv" -Execute
& $Gen collect $r.World "$Work\probes2.tsv" "$Work\results2.tsv"
& $Gen build "$Work\probes1.tsv" "$Work\results1.tsv" "$Work\probes2.tsv" "$Work\results2.tsv" $Version $Out

# 4. Int states that reached the probe limit are probed further.
& $Gen probes3 $Out "$Work\probes3.tsv"
if ((Get-Content "$Work\probes3.tsv" -ErrorAction SilentlyContinue).Count -gt 0) {
  $r = Probe "ph3" "$Work\probes3.tsv" -Execute
  & $Gen collect $r.World "$Work\probes3.tsv" "$Work\results3.tsv"
  & $Gen extend $Out "$Work\probes3.tsv" "$Work\results3.tsv" $Out
}
& $Gen audit $Out
