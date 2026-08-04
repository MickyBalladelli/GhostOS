#!/usr/bin/env python3
"""Send one bounded shell command to a SynOS QEMU console through QMP."""

from __future__ import annotations

import json
import socket
import sys
import time


KEYS = {
    " ": "spc",
    "/": "slash",
    "-": "minus",
    "_": "shift-minus",
    "=": "equal",
    ".": "dot",
    ":": "shift-semicolon",
}


def read_message(stream: socket.socket) -> None:
    stream.settimeout(1.0)
    try:
        stream.recv(4096)
    except TimeoutError:
        pass


def send(stream: socket.socket, payload: dict) -> None:
    stream.sendall((json.dumps(payload) + "\r\n").encode())
    read_message(stream)


def key_name(character: str) -> str:
    if character.isalpha() or character.isdigit():
        return character.lower()
    try:
        return KEYS[character]
    except KeyError as error:
        raise SystemExit(f"unsupported QEMU key: {character!r}") from error


def main() -> int:
    if len(sys.argv) != 3:
        raise SystemExit(f"usage: {sys.argv[0]} QMP_SOCKET COMMAND")

    socket_path, command = sys.argv[1:]
    with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as stream:
        deadline = time.monotonic() + 10
        while True:
            try:
                stream.connect(socket_path)
                break
            except (FileNotFoundError, ConnectionRefusedError):
                if time.monotonic() >= deadline:
                    raise
                time.sleep(0.05)
        read_message(stream)
        send(stream, {"execute": "qmp_capabilities"})
        for character in command:
            send(
                stream,
                {
                    "execute": "human-monitor-command",
                    "arguments": {"command-line": f"sendkey {key_name(character)}"},
                },
            )
        send(
            stream,
            {
                "execute": "human-monitor-command",
                "arguments": {"command-line": "sendkey ret"},
            },
        )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
