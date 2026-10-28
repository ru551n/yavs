# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this file,
# You can obtain one at http://mozilla.org/MPL/2.0/.
#
# Copyright (c) 2014-2026, Lars Asplund lars.anders.asplund@gmail.com

"""
A jobserver lending the spare CPU of a parallel run (-p N) to the yavs simulations.

Each running test gets a private GNU make jobserver, a named pipe passed as
MAKEFLAGS=--jobserver-auth=fifo:PATH, and a distributor thread lends tokens into the pipes.
See "Jobserver" in the package README.
"""

import logging
import itertools
import os
import shutil
import tempfile
import threading

LOGGER = logging.getLogger(__name__)
TOKEN = b"+"


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
    A running test and its private jobserver pipe
    """

    def __init__(self, path, cap):
        self.path = path
        self.cap = cap  # Most tokens to lend, None for no cap
        os.mkfifo(path)
        # Read-write, so writing never blocks or fails for want of a reader, and the child's open never blocks
        self.fd = os.open(path, os.O_RDWR | os.O_NONBLOCK)
        self.lent = 0  # Tokens written and not read back: idle in the pipe or in use by the simulation
        self.idle = False  # Left tokens unread at the last tick: it cannot use more

    def give(self, count):
        os.write(self.fd, TOKEN * count)
        self.lent += count

    def reclaim(self):
        """Take back the idle tokens in the pipe and return how many"""
        try:
            count = len(os.read(self.fd, self.lent)) if self.lent else 0
        except BlockingIOError:
            count = 0
        self.lent -= count
        return count

    def limit(self):
        """The most tokens to hold now: one more than in use while it leaves tokens unread"""
        if self.idle:
            return self.lent + 1 if self.cap is None else min(self.cap, self.lent + 1)
        return self.cap

    def close(self):
        os.close(self.fd)
        os.unlink(self.path)


class Jobserver:
    """
    The CPU budget of a run with num_threads workers, make style: one running test is free and each further
    running test takes one slot, so with k tests running num_threads - k tokens are lent to the simulations.
    A test starting while every token is in use runs at once, so the total is num_threads in the steady state,
    not at every instant.
    """

    def __init__(self, num_threads, interval=0.05):
        self.num_threads = num_threads
        self._clients = []  # Oldest first
        self._interval = interval
        self._lock = threading.Lock()
        self._stop = threading.Event()
        self._thread = None
        # MAKEFLAGS is split at spaces, so the pipes live where the path has none, not in the output path
        self._dir = tempfile.mkdtemp(prefix="vunit-yavs-jobserver-")
        if " " in self._dir:
            shutil.rmtree(self._dir)
            raise RuntimeError(
                f"The jobserver pipes need a directory without spaces, but the temporary directory is {self._dir}. "
                "Set TMPDIR to a path without spaces, or drop --yavs-jobserver."
            )
        self._names = itertools.count()

    def env(self, client, env):
        """The environment of the simulation of client: MAKEFLAGS replaced, never an outer make's passed on"""
        env = dict(os.environ if env is None else env)
        env["MAKEFLAGS"] = f"-j{self.num_threads} --jobserver-auth=fifo:{client.path}"
        return env

    def add(self, cap=None):
        """Register a starting test with a pipe of its own, and lend to it"""
        with self._lock:
            client = Client(os.path.join(self._dir, f"fifo{next(self._names)}"), cap)
            self._clients.append(client)
            self._rebalance(tick=False)
            if self._thread is None:
                self._thread = threading.Thread(target=self._run, daemon=True)
                self._thread.start()
        return client

    def remove(self, client):
        """Unregister a test whose simulation ended; tokens it never returned go back to the pool"""
        with self._lock:
            client.reclaim()
            if client.lent:
                LOGGER.warning(
                    "The yavs simulation using jobserver %s exited holding %i token(s) without returning them; "
                    "they were put back in the pool. A jobserver client must write back every token it reads.",
                    client.path,
                    client.lent,
                )
            client.close()
            self._clients.remove(client)
            self._rebalance(tick=False)

    def _run(self):
        while not self._stop.wait(self._interval):
            with self._lock:
                self._rebalance(tick=True)

    def _rebalance(self, tick):
        """Take back every idle token and lend the free ones again by priority"""
        for client in self._clients:
            reclaimed = client.reclaim()
            if tick:
                client.idle = reclaimed > 0
        # Negative while a new test runs on tokens still in use elsewhere: lend nothing until they return
        free = self.num_threads - max(len(self._clients), 1) - sum(client.lent for client in self._clients)
        held = [client.lent for client in self._clients]
        limits = [client.limit() for client in self._clients]
        for client, count in zip(self._clients, lend_order(free, held, limits)):
            if count:
                client.give(count)

    def close(self):
        """Stop lending and warn about tests still holding tokens (the end of run leak check)"""
        self._stop.set()
        if self._thread is not None:
            self._thread.join()
        with self._lock:
            for client in self._clients:
                client.reclaim()
            held = {client.path: client.lent for client in self._clients if client.lent}
            for client in self._clients:
                client.close()
            self._clients = []
            shutil.rmtree(self._dir, ignore_errors=True)
        if held:
            LOGGER.warning(
                "The jobserver is missing %i of its %i tokens at the end of the run: "
                "simulations still running hold them (pipe: tokens): %s.",
                sum(held.values()),
                self.num_threads - 1,
                held,
            )
