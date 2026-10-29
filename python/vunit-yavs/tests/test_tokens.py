# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this file,
# You can obtain one at http://mozilla.org/MPL/2.0/.
#
# Copyright (c) 2014-2026, Lars Asplund lars.anders.asplund@gmail.com

"""
Test the token server lending the spare CPU of a parallel run to the simulations
"""

# pylint: disable=protected-access

import os
import socket
import subprocess
import sys
import time
import unittest

from vunit_yavs.tokens import ENV, TokenServer, lend_order

# A simulation in a child process: takes what it wants, prints it, then waits to be killed
CHILD = r"""
import os, socket, sys
port, secret = os.environ["YAVS_TOKENS"].split(":")
sock = socket.create_connection(("127.0.0.1", int(port)))
stream = sock.makefile("rb")
sock.sendall(secret.encode() + b"\n")
assert stream.readline() == b"0 0\n"
sock.sendall(b"3\n")
print(stream.readline().decode().strip(), flush=True)
sys.stdin.read()
"""


class Sim:
    """A simulation's end of the protocol"""

    def __init__(self, server, client, secret=None):
        self.sock = socket.create_connection(("127.0.0.1", server.port), timeout=10)
        self.stream = self.sock.makefile("rb")
        self.sock.sendall((client.secret if secret is None else secret).encode() + b"\n")

    def line(self):
        return self.stream.readline().decode()

    def want(self, count):
        """Send a want line and return the answer to it, (message, lent)"""
        self.sock.sendall(f"{count}\n".encode())
        message, lent = self.line().split()
        return int(message), int(lent)

    def close(self):
        self.stream.close()
        self.sock.close()


def wait_for(condition):
    deadline = time.monotonic() + 10
    while not condition():
        assert time.monotonic() < deadline, "timed out"
        time.sleep(0.005)


class TestTokenServer(unittest.TestCase):
    """
    Test the token server over real sockets
    """

    def setUp(self):
        self.server = None

    def tearDown(self):
        if self.server is not None:
            self.server.close()

    def start(self, num_threads=None, cap=None):
        """Register a test and connect its simulation"""
        if self.server is None:
            self.server = TokenServer(num_threads)
        client = self.server.add(cap)
        sim = Sim(self.server, client)
        self.addCleanup(sim.close)
        self.assertEqual(sim.line(), "0 0\n")
        return client, sim

    def lent(self):
        with self.server._lock:
            return [conn.lent for client in self.server._clients for conn in client.conns]

    def test_lend_order_breadth_first_newest_first(self):
        # 4 tokens for A (oldest), B, C (newest): C, B, A get one each, then C the fourth
        self.assertEqual(lend_order(4, [0, 0, 0]), [1, 1, 2])
        self.assertEqual(lend_order(3, [2, 0]), [0, 3])
        self.assertEqual(lend_order(2, [1, 0, 0]), [0, 1, 1])
        self.assertEqual(lend_order(0, [0, 0]), [0, 0])
        self.assertEqual(lend_order(-2, [0, 0]), [0, 0])
        self.assertEqual(lend_order(3, []), [])

    def test_lend_order_respects_limits(self):
        self.assertEqual(lend_order(6, [0, 0, 0], [1, None, 1]), [1, 4, 1])
        self.assertEqual(lend_order(6, [0, 0], [1, 2]), [1, 2])
        self.assertEqual(lend_order(2, [3, 0], [3, 0]), [0, 0])

    def test_env_names_the_port_and_the_secret(self):
        self.server = TokenServer(4)
        client = self.server.add()
        self.assertEqual(self.server.env(client, {"A": "1"}), {"A": "1", ENV: f"{self.server.port}:{client.secret}"})
        self.assertNotEqual(self.server.add().secret, client.secret)

    def test_lends_up_to_the_want_and_answers_every_message(self):
        _, sim = self.start(8)
        self.assertEqual(sim.want(3), (1, 3))
        self.assertEqual(sim.want(0), (2, 0))
        self.assertEqual(sim.want(9), (3, 7))
        self.assertEqual(self.lent(), [7])

    def test_a_wrong_secret_is_closed_unanswered(self):
        self.server = TokenServer(4)
        client = self.server.add()
        sim = Sim(self.server, client, secret="wrong")
        self.addCleanup(sim.close)
        self.assertEqual(sim.line(), "")
        self.assertEqual(self.lent(), [])

    def test_a_secret_of_an_ended_test_is_closed(self):
        self.server = TokenServer(4)
        client = self.server.add()
        self.server.remove(client)
        sim = Sim(self.server, client)
        self.addCleanup(sim.close)
        self.assertEqual(sim.line(), "")

    def test_total_cpu_slots_stay_num_threads(self):
        # -p 16 with 4 tests running: 12 free slots; the first to ask gets them all, 16 in all, never more
        sims = [self.start(16)[1] for _ in range(4)]
        for sim in sims:
            sim.want(15)
        self.assertEqual(self.lent(), [12, 0, 0, 0])
        # What it gives back goes breadth first to the others
        sims[0].want(3)
        wait_for(lambda: self.lent() == [3, 3, 3, 3])
        self.assertEqual(4 + sum(self.lent()), 16)

    def test_breadth_first_newest_first(self):
        # -p 7 with 3 tests: 4 free tokens; wants arrive oldest first and nothing is taken back
        sims = [self.start(7)[1] for _ in range(3)]
        self.assertEqual(sims[0].want(6), (1, 4))
        self.assertEqual(sims[1].want(6), (1, 0))
        sims[0].want(1)  # Gives 3 back: breadth first, newest first
        wait_for(lambda: self.lent() == [1, 3, 0])
        self.assertEqual(sims[2].want(6), (1, 0))
        sims[1].want(0)
        wait_for(lambda: self.lent() == [1, 0, 3])

    def test_tokens_in_use_are_not_taken_back(self):
        _, first = self.start(7)
        self.assertEqual(first.want(6), (1, 6))
        _, second = self.start()
        self.assertEqual(second.want(6), (1, 0))
        self.assertEqual(self.lent(), [6, 0])

    def test_cap_limits_the_tokens_of_a_test(self):
        _, first = self.start(8, cap=1)
        _, second = self.start(cap=None)
        self.assertEqual(first.want(7), (1, 1))
        self.assertEqual(second.want(7), (1, 5))

    def test_more_is_lent_as_tests_end_with_the_current_message_number(self):
        _, first = self.start(4)
        second_client, second = self.start()
        third_client, third = self.start()
        self.assertEqual(first.want(3), (1, 1))
        self.assertEqual(first.want(3), (2, 1))
        self.server.remove(second_client)
        self.assertEqual(first.line(), "2 2\n")
        self.server.remove(third_client)
        self.assertEqual(first.line(), "2 3\n")
        self.assertEqual(second.line(), "")  # Its connection was closed
        self.assertEqual(third.line(), "")

    def test_a_closed_connection_gives_its_tokens_back(self):
        _, first = self.start(4)
        _, second = self.start()
        self.assertEqual(first.want(3), (1, 2))
        self.assertEqual(second.want(3), (1, 0))
        first.close()
        self.assertEqual(second.line(), "1 2\n")

    def test_a_garbled_line_closes_the_connection(self):
        _, first = self.start(4)
        _, second = self.start()
        self.assertEqual(first.want(3), (1, 2))
        first.sock.sendall(b"many\n")
        self.assertEqual(first.line(), "")
        self.assertEqual(second.want(3), (1, 2))

    def test_a_killed_simulation_gives_its_tokens_back(self):
        client, sim = self.start(4)
        child = self.server.add()
        proc = subprocess.Popen(  # pylint: disable=consider-using-with
            [sys.executable, "-c", CHILD],
            env=self.server.env(child, os.environ),
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            text=True,
        )
        self.addCleanup(proc.wait)
        self.addCleanup(proc.kill)
        self.assertEqual(proc.stdout.readline().strip(), "1 2")
        self.assertEqual(sim.want(3), (1, 0))
        proc.kill()  # Unread lines in its socket: a reset rather than an end of stream on some systems
        proc.wait()
        self.assertEqual(sim.line(), "1 2\n")
        self.server.remove(child)
        self.assertEqual(sim.line(), "1 3\n")
        self.server.remove(client)

    def test_close_stops_the_server(self):
        self.server = TokenServer(4)
        port = self.server.port
        self.server.close()
        self.server = None
        with self.assertRaises(OSError):
            socket.create_connection(("127.0.0.1", port), timeout=5).close()
