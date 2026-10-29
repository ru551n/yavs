# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this file,
# You can obtain one at http://mozilla.org/MPL/2.0/.
#
# Copyright (c) 2014-2026, Lars Asplund lars.anders.asplund@gmail.com

"""
A token server lending the spare CPU of a parallel run (-p N) to the yavs simulations.

It listens on 127.0.0.1 at a free port. Each running test gets a secret of its own, passed with the port as
YAVS_TOKENS=<port>:<secret>. A simulation connects, sends the secret (answered with "0 0"), then a line
"<want>" whenever the extra threads it could use change; every such line, the k-th, is answered with
"<k> <lent>" and again whenever more is lent for it. A closed connection gives every token back.
See "Token server" in the package README.
"""

import hmac
import logging
import secrets
import selectors
import socket
import threading

LOGGER = logging.getLogger(__name__)
ENV = "YAVS_TOKENS"


def lend_order(free, held, limits=None):
    """
    Return how many of ``free`` tokens to lend each client, given the tokens each client holds,
    oldest client first, and optionally the most tokens each client may hold.

    Breadth first, since a second thread rarely doubles the speed of one test, and newest test
    first within a round, since old tests usually finish first and free their tokens anyway.
    """
    lend = [0] * len(held)
    limits = [None] * len(held) if limits is None else limits
    for _ in range(max(free, 0)):
        open_ = [i for i in reversed(range(len(held))) if limits[i] is None or held[i] + lend[i] < limits[i]]
        if not open_:
            break
        lend[min(open_, key=lambda i: held[i] + lend[i])] += 1
    return lend


class Client:
    """
    A running test: its secret, the most tokens it may hold, and the connections of its simulation
    """

    def __init__(self, cap):
        self.secret = secrets.token_hex(16)
        self.cap = cap  # None for no cap
        self.conns = []


class Connection:
    """
    One connection of a simulation
    """

    def __init__(self, sock):
        self.sock = sock
        self.buf = b""
        self.client = None  # Set by the secret
        self.messages = 0  # Want lines received
        self.want = 0
        self.lent = 0
        self.dead = False

    def limit(self):
        return self.want if self.client.cap is None else min(self.want, self.client.cap)


class TokenServer:
    """
    The CPU budget of a run with num_threads workers, make style: one running test is free and each further
    running test takes one slot, so with k tests running num_threads - k tokens are lent to the simulations.
    Tokens in use are never taken back: a test starting while every token is in use runs at once, so the
    total is num_threads in the steady state, not at every instant.
    """

    def __init__(self, num_threads):
        self.num_threads = num_threads
        self._clients = []  # Oldest first
        self._lock = threading.Lock()
        self._stop = threading.Event()
        self._listener = socket.create_server(("127.0.0.1", 0))
        self._listener.setblocking(False)
        self.port = self._listener.getsockname()[1]
        self._selector = selectors.DefaultSelector()
        self._selector.register(self._listener, selectors.EVENT_READ)
        self._thread = threading.Thread(target=self._run, daemon=True)
        self._thread.start()

    def env(self, client, env):
        """The environment of the simulation of client"""
        env = dict(env)
        env[ENV] = f"{self.port}:{client.secret}"
        return env

    def add(self, cap=None):
        """Register a starting test"""
        client = Client(cap)
        with self._lock:
            self._clients.append(client)
        return client

    def remove(self, client):
        """Unregister a test whose simulation ended: its connections close and their tokens come back"""
        with self._lock:
            for conn in client.conns:
                self._kill(conn)
            self._clients.remove(client)
            self._send(self._rebalance())

    def close(self):
        """Stop serving"""
        self._stop.set()
        self._thread.join()

    def _run(self):
        while not self._stop.is_set():
            for key, _ in self._selector.select(timeout=0.1):
                if key.data is None:
                    self._accept()
                else:
                    self._read(key.data)
        for key in list(self._selector.get_map().values()):
            key.fileobj.close()
        self._selector.close()

    def _accept(self):
        try:
            sock, _ = self._listener.accept()
        except OSError:
            return
        sock.setblocking(False)
        sock.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)
        self._selector.register(sock, selectors.EVENT_READ, Connection(sock))

    def _read(self, conn):
        try:
            data = conn.sock.recv(4096)
        except BlockingIOError:
            return
        except OSError:  # A reset: a simulation killed with replies unread
            data = b""
        with self._lock:
            conn.buf += data
            while not conn.dead and b"\n" in conn.buf:
                line, conn.buf = conn.buf.split(b"\n", 1)
                self._line(conn, line.strip())
            if not data or conn.dead or len(conn.buf) > 256:
                self._kill(conn)
                self._selector.unregister(conn.sock)
                conn.sock.close()
                self._send(self._rebalance())

    def _line(self, conn, line):
        if conn.client is None:
            conn.client = next(
                (c for c in self._clients if hmac.compare_digest(c.secret.encode(), line)),
                None,
            )
            if conn.client is None:
                conn.dead = True
            else:
                conn.client.conns.append(conn)
                self._send([conn])  # "0 0"
            return
        if not line.isdigit():
            conn.dead = True
            return
        conn.messages += 1
        conn.want = int(line)
        conn.lent = min(conn.lent, conn.want)
        self._send([conn] + [c for c in self._rebalance() if c is not conn])

    def _kill(self, conn):
        """Take conn out of the accounting; the selector thread closes it"""
        conn.dead = True
        conn.lent = 0
        if conn.client is not None and conn in conn.client.conns:
            conn.client.conns.remove(conn)
        try:
            conn.sock.shutdown(socket.SHUT_RDWR)
        except OSError:
            pass

    def _rebalance(self):
        """Lend the free tokens by priority and return the connections lent more"""
        conns = [conn for client in self._clients for conn in client.conns]
        free = self.num_threads - max(len(self._clients), 1) - sum(conn.lent for conn in conns)
        changed = []
        for conn, count in zip(conns, lend_order(free, [conn.lent for conn in conns], [c.limit() for c in conns])):
            if count:
                conn.lent += count
                changed.append(conn)
        return changed

    def _send(self, conns):
        for conn in conns:
            try:
                conn.sock.sendall(f"{conn.messages} {conn.lent}\n".encode())
            except OSError:
                self._kill(conn)
