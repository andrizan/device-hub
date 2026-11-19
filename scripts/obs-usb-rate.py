"""Publish the last second of USB mouse IN completions to an OBS text source."""

import argparse
from collections import deque
import os
from pathlib import Path
import shutil
import struct
import subprocess
import sys
from tempfile import NamedTemporaryFile
import threading
import time

PCAP_HEADER = struct.Struct("<IHHIIII")
RECORD = struct.Struct("<IIII")
USB = struct.Struct("<HQIHBHHBBI")
SECOND_US = 1_000_000


def capture_times(stream, device, endpoint, report_id):
    header = stream.read(PCAP_HEADER.size)
    if len(header) != PCAP_HEADER.size:
        raise ValueError("USBPcap did not return a capture header")
    magic, major, minor, _, _, snaplen, link = PCAP_HEADER.unpack(header)
    if (magic, major, minor, link) != (0xA1B2C3D4, 2, 4, 249):
        raise ValueError("Expected a little-endian USBPcap capture")
    while raw := stream.read(RECORD.size):
        if len(raw) != RECORD.size:
            raise ValueError("Incomplete capture record")
        sec, us, size, _ = RECORD.unpack(raw)
        if not USB.size <= size <= min(snaplen, 65535):
            raise ValueError("Invalid USBPcap packet size")
        packet = stream.read(size)
        if len(packet) != size:
            raise ValueError("Incomplete USBPcap packet")
        offset, _, status, _, info, _, addr, ep, transfer, length = USB.unpack_from(packet)
        if (addr, ep, transfer, status) != (device, endpoint, 1, 0):
            continue
        if info & 1 and length and USB.size <= offset < size:
            if report_id is None or packet[offset] == report_id:
                yield sec * SECOND_US + us


class Rate:
    def __init__(self):
        self.samples = deque()
        self.seen = False
        self.lock = threading.Lock()

    def record(self, stamp):
        with self.lock:
            self.seen = True
            self.samples.append(stamp)
            self.expire(stamp)

    def expire(self, now):
        while self.samples and self.samples[0] <= now - SECOND_US:
            self.samples.popleft()

    def label(self, now):
        with self.lock:
            self.expire(now)
            return f"USB  {len(self.samples)} Hz" if self.seen else "USB  -- Hz"


def publish(path, label):
    with NamedTemporaryFile(dir=path.parent, delete=False) as file:
        temp = Path(file.name)
        file.write(label.encode("utf-8"))
    try:
        temp.replace(path)
    finally:
        temp.unlink(missing_ok=True)


def run(args):
    path = Path(os.environ["APPDATA"]) / "DeviceHub" / "obs_usb_rate.txt"
    path.parent.mkdir(parents=True, exist_ok=True)
    publish(path, "USB  -- Hz")
    print(f"OBS text file: {path}", flush=True)
    program = shutil.which("USBPcapCMD.exe") or str(
        Path(os.environ["ProgramFiles"]) / "USBPcap" / "USBPcapCMD.exe"
    )
    rate = Rate()
    done = threading.Event()
    errors = []
    cmd = [program, "-d", rf"\\.\USBPcap{args.hub}", "--devices", str(args.device),
           "-s", "64", "-o", "-"]
    with subprocess.Popen(cmd, stdout=subprocess.PIPE) as proc:
        def read_packets():
            try:
                for stamp in capture_times(proc.stdout, args.device, args.endpoint, args.report_id):
                    rate.record(stamp)
            except (OSError, ValueError) as exc:
                errors.append(exc)
            finally:
                done.set()

        reader = threading.Thread(target=read_packets)
        reader.start()
        try:
            previous = "USB  -- Hz"
            while not done.wait(0.25):
                label = rate.label(time.time_ns() // 1000)
                if previous != label:
                    try:
                        publish(path, label)
                    except PermissionError:
                        continue  # OBS may have the file open. Retry at the next tick.
                    previous = label
            if errors:
                raise errors[0]
            raise RuntimeError("USBPcap stopped. Check the device and capture permissions.")
        finally:
            if proc.poll() is None:
                proc.terminate()
            proc.wait()
            reader.join()
            publish(path, "USB  -- Hz")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--hub", type=int, required=True, help="N in USBPcapN")
    parser.add_argument("--device", type=int, required=True, help="USB device address")
    parser.add_argument("--endpoint", type=lambda s: int(s, 0), required=True, help="mouse IN endpoint, e.g. 0x81")
    parser.add_argument("--report-id", type=lambda s: int(s, 0), help="mouse report ID for a shared endpoint")
    args = parser.parse_args()
    if args.hub < 1 or not 1 <= args.device <= 127 or not 0x81 <= args.endpoint <= 0x8F:
        parser.error("Expected a positive hub number, device 1..127, and IN endpoint 0x81..0x8f")
    if args.report_id is not None and not 0 <= args.report_id <= 255:
        parser.error("Report ID must fit in one byte")
    try:
        run(args)
    except KeyboardInterrupt:
        pass
    except (OSError, ValueError, RuntimeError) as exc:
        sys.exit(str(exc))
