# Count successful interrupt IN completions on one mouse endpoint.
[CmdletBinding(DefaultParameterSetName = 'Live')]
param(
    [Parameter(Mandatory, ParameterSetName = 'Live')]
    [ValidatePattern('USBPcap[0-9]+$')][string]$Interface,
    [Parameter(Mandatory, ParameterSetName = 'File')]
    [string]$CaptureFile,
    [Parameter(Mandatory)][ValidateRange(0, 65535)][int]$Bus,
    [Parameter(Mandatory)][ValidateRange(1, 127)][int]$Device,
    [Parameter(Mandatory)][ValidateRange(129, 143)][int]$Endpoint,
    [Parameter(ParameterSetName = 'Live')][ValidateRange(2, 300)][int]$Seconds = 10
)
$ErrorActionPreference = 'Stop'
$tshark = (Get-Command tshark.exe -ErrorAction SilentlyContinue).Source
if (!$tshark) {
    $tshark = Join-Path $env:ProgramFiles 'Wireshark\tshark.exe'
    if (!(Test-Path -LiteralPath $tshark)) {
        throw 'Install Wireshark with TShark. Live measurements also require USBPcap.'
    }
}
if ($PSCmdlet.ParameterSetName -eq 'File') {
    $sourceArgs = @('-r', (Resolve-Path -LiteralPath $CaptureFile).Path)
} else {
    $sourceArgs = @('-i', $Interface, '-a', "duration:$Seconds")
}
$filter = "usb.bus_id == $Bus && usb.device_address == $Device && usb.endpoint_address == $Endpoint && usb.transfer_type == 1 && usb.irp_info.direction == 1 && usb.usbd_status == 0 && usb.data_len > 0"
# TShark aggregates by capture timestamp, independently of stdout scheduling.
& $tshark @sourceArgs -n -q -z "io,stat,1,$filter"
if ($LASTEXITCODE -ne 0) {
    throw "TShark failed (exit $LASTEXITCODE). Check the capture interface and capture permissions."
}
