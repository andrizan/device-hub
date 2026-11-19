# How to use Device HUB v0.3.0 (Windows x64)

## Download and launch

Get the ZIP that suits your purpose from the Assets of the [GitHub Release](https://github.com/pity-zxcv/device-hub/releases/tag/v0.3.0).

| Purpose | ZIP | Extract to |
|---|---|---|
| App and OBS plugin | `device-hub-v0.3.0-windows-x64.zip` | Any folder |
| OBS plugin only | `devhub-obs-v0.3.0-plugin.zip` | `%ProgramData%\obs-studio\plugins\devhub_obs\` |

To verify a download, run `Get-FileHash .\filename.zip -Algorithm SHA256` in PowerShell and compare the result with `SHA256SUMS.txt` of the same release.

For the app, extract the ZIP and start `device-hub.exe`. Connect the device you want to configure with a USB cable or a supported dongle, close the manufacturer's configuration site, and then choose the device on the Home page. Settings such as DPI, button assignment, keyboard, and EQ are shown according to the selected product.

## Measure the mouse with the game in the foreground

1. On the PC where you will measure USB transfers, install the [USBPcap driver](https://desowin.org/usbpcap/) and restart Windows. Python, Wireshark, and TShark are not needed. Installing USBPcap is optional if you only want normal device settings.
2. Start Device HUB as administrator, and on the mouse's "Test" page choose "USB transfer (USBPcap)". Select the target product name. A wireless mouse may be listed under the name of its USB dongle.
3. Press "Auto-detect EP" and move only the target mouse for 3 seconds. If you note the detected EP number, you can choose the same target in OBS too. When entering it by hand, `0x81` is `1`, and a Report ID of `-1` means all.
4. Start measuring, put the game in the foreground, and move the mouse for a few seconds. When you go back to the app and stop, you can see the largest number of transfers in any consecutive 1 second during the measurement.

The current value is the number of successful interrupt IN completions with data on the target EP in the last 1 second. For a mouse that pauses sending while idle, the value drops. This value does not reveal the product's configured Hz or the number of polls on the bus that returned no data. Mice connected over Bluetooth are not supported.

## Show it live in OBS

1. Close OBS. For the full build, copy the extracted `devhub_obs` folder to `%ProgramData%\obs-studio\plugins\`. For the standalone build, create the extraction location from the table above and extract the contents of the ZIP into it. The final DLL location is `%ProgramData%\obs-studio\plugins\devhub_obs\bin\64bit\devhub_obs.dll`. Place the adjacent `data\locale` together with it.
2. Stop USB measurement on the app side, and start 64-bit OBS Studio as administrator. You do not need to keep the Device HUB app itself open while streaming.
3. Add "Sources" → "+" → "Device HUB USB Polling Rate" to the scene. In the properties, choose the mouse or dongle under "Target device (product)", and set the IN endpoint number and the Report ID. Turn on "Measure USB input".
4. Move the mouse, check that the Hz in the preview changes, and move the display to a position you like. To show it in other scenes as well, use the same source with "Add Existing".

| Symptom | What to check |
|---|---|
| Not in the source list | Check that the DLL folder is not nested twice, and restart OBS. |
| driver / hub not found | Check the restart after installing the driver and the USB connection of the target. |
| access denied / hub already in use | Start the measuring app as administrator, and stop any other source, app, or Wireshark that is measuring the same hub. |
| Waiting for mouse input | Move the target. If nothing changes, re-check the product, EP, and Report ID. |
| No response after unplugging/replugging | Stop measuring, refresh the product list, and select the target again. In OBS, select it again in the properties and restart measurement. |
