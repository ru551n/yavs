# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this file,
# You can obtain one at http://mozilla.org/MPL/2.0/.
#
# Copyright (c) 2014-2026, Lars Asplund lars.anders.asplund@gmail.com

"""
Test the jobserver lending the spare CPU of a parallel run to the simulations
"""

# pylint: disable=protected-access

import os
import sys
import unittest

from vunit.ostools import Process

from vunit_yavs.jobserver import Jobserver, lend_order

CHILD = r"""
import os, sys
flags = os.environ["MAKEFLAGS"]
print(flags)
fd = os.open(flags.split("--jobserver-auth=fifo:")[1], os.O_RDWR)
token = os.read(fd, 1)
print("token", token.decode())
if sys.argv[1] == "return":
    os.write(fd, token)
"""


@unittest.skipIf(sys.platform == "win32", "The jobserver needs POSIX named pipes")
class TestJobserver(unittest.TestCase):
    """
    Test the jobserver
    """

    def setUp(self):
        self.jobserver = None
        self.clients = []

    def tearDown(self):
        if self.jobserver is not None:
            self.jobserver.close()

    def start(self, num_threads=None, cap=None):
        """Start a test on a jobserver that does not tick on its own"""
        if self.jobserver is None:
            self.jobserver = Jobserver(num_threads, interval=3600)
        self.clients.append(self.jobserver.add(cap))
        return self.clients[-1]

    def lent(self):
        return [client.lent for client in self.jobserver._clients]

    def take(self, client, count):
        """Read count tokens as the simulation would"""
        fd = os.open(client.path, os.O_RDWR)
        try:
            return os.read(fd, count)
        finally:
            os.close(fd)

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

    def test_total_cpu_slots_stay_num_threads(self):
        # -p 16 with 4 tests running: 12 free slots, 3 extra threads for each, 16 in all, never 19
        for _ in range(4):
            self.start(16)
        self.assertEqual(self.lent(), [3, 3, 3, 3])
        self.assertEqual(4 + sum(self.lent()), 16)

    def test_new_tests_get_tokens_first(self):
        self.start(7)
        self.assertEqual(self.lent(), [6])
        self.start()
        self.assertEqual(self.lent(), [2, 3])
        self.start()
        self.assertEqual(self.lent(), [1, 1, 2])

    def test_tokens_in_use_are_not_taken_back(self):
        first = self.start(7)
        self.assertEqual(self.take(first, 2), b"++")
        self.start()
        self.assertEqual(self.lent(), [2, 3])

    def test_cap_limits_the_tokens_of_a_test(self):
        self.start(8, cap=1)
        self.start(cap=None)
        self.assertEqual(self.lent(), [1, 5])

    def test_tokens_left_unread_are_lent_one_at_a_time(self):
        # A test that cannot use more threads parks no more than one token
        first = self.start(7)
        self.start()
        self.start()
        self.assertEqual(self.lent(), [1, 1, 2])
        self.assertEqual(self.take(first, 1), b"+")
        self.jobserver._rebalance(tick=True)
        self.assertEqual(self.lent(), [2, 1, 1])
        self.jobserver._rebalance(tick=True)
        self.assertEqual(self.lent(), [2, 1, 1])

    def test_tokens_return_when_a_test_finishes(self):
        for _ in range(3):
            self.start(7)
        self.jobserver.remove(self.clients[2])
        self.assertEqual(self.lent(), [2, 3])
        self.assertFalse(os.path.exists(self.clients[2].path))

    def test_warns_and_restores_tokens_of_a_test_that_exited_holding_them(self):
        client = self.start(3)
        self.assertEqual(self.take(client, 1), b"+")
        with self.assertLogs("vunit_yavs.jobserver", level="WARNING") as logs:
            self.jobserver.remove(client)
        self.assertIn("exited holding 1 token(s)", logs.output[0])
        self.assertEqual(self.lent(), [])
        self.start()
        self.assertEqual(self.lent(), [2])

    def test_pipes_live_in_a_directory_of_their_own(self):
        client = self.start(3)
        self.assertNotIn(" ", client.path)
        directory = os.path.dirname(client.path)
        self.jobserver.close()
        self.assertFalse(os.path.exists(directory))

    def test_close_warns_unless_every_token_came_back(self):
        client = self.start(3)
        self.take(client, 1)
        with self.assertLogs("vunit_yavs.jobserver", level="WARNING") as logs:
            self.jobserver.close()
        self.assertIn("missing 1 of its 2 tokens", logs.output[0])

        self.jobserver, self.clients = None, []
        self.start(3)
        with self.assertNoLogs("vunit_yavs.jobserver", level="WARNING"):
            self.jobserver.close()

    def run_child(self, mode):
        """Run CHILD with the environment of a new test and return its output and client"""
        client = self.start(4)
        env = self.jobserver.env(client, dict(os.environ, MAKEFLAGS="-j32 --jobserver-auth=3,4"))
        output = []
        Process([sys.executable, "-c", CHILD, mode], env=env).consume_output(output.append)
        return output, client

    def test_child_takes_and_returns_a_token(self):
        output, client = self.run_child("return")
        # An outer make's MAKEFLAGS is replaced, not passed on
        self.assertEqual(output[0], f"-j4 --jobserver-auth=fifo:{client.path}")
        self.assertEqual(output[1], "token +")
        with self.assertNoLogs("vunit_yavs.jobserver", level="WARNING"):
            self.jobserver.remove(client)

    def test_child_keeping_a_token_is_reported(self):
        _, client = self.run_child("keep")
        with self.assertLogs("vunit_yavs.jobserver", level="WARNING"):
            self.jobserver.remove(client)

    def test_distributor_thread_reclaims_and_relends(self):
        self.jobserver = Jobserver(3, interval=0.01)
        client = self.jobserver.add()
        self.assertEqual(client.lent, 2)
        self.jobserver._stop.wait(0.2)
        with self.jobserver._lock:
            self.assertEqual(client.lent, 1)  # Unread tokens: one at a time
        self.jobserver.close()
