#!/usr/bin/env python3
"""Serve local frontend fixtures reliably for parallel browser tests."""

from functools import partial
from http.server import SimpleHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path


class FrontendTestServer(ThreadingHTTPServer):
    # Parallel browsers open several asset connections at once. The default
    # queue of five can reset requests, leaving scripts and styles unloaded.
    request_queue_size = 128


if __name__ == "__main__":
    frontend = Path(__file__).resolve().parents[1] / "frontend"
    handler = partial(SimpleHTTPRequestHandler, directory=str(frontend))
    with FrontendTestServer(("127.0.0.1", 4173), handler) as server:
        server.serve_forever()
