#!/usr/bin/env python3

import subprocess
import tempfile
import os
import unittest
from test_base import QsvTestBase

class TestTimeline(QsvTestBase):
    
    def test_timeline_basic(self):
        """Test basic timeline functionality"""
        result = self.run_qsv_command(f"load {self.get_fixture_path('simple_timeline.csv')} - timeline datetime --interval 1h - show")
        self.assertEqual(result.stdout.strip(), "\n".join([
            "timeline_1h,count",
            "2023-01-01 00:00:00,1",
            "2023-01-01 01:00:00,2",
            "2023-01-01 02:00:00,3",
            "2023-01-01 03:00:00,4",
            "2023-01-01 04:00:00,5",
            "2023-01-01 05:00:00,6",
            "2023-01-01 06:00:00,7",
            "2023-01-01 07:00:00,8",
            "2023-01-01 08:00:00,9",
            "2023-01-01 09:00:00,10",
            "2023-01-01 10:00:00,11",
            "2023-01-01 11:00:00,12",
            "2023-01-01 12:00:00,13",
        ]))

    def test_timeline_with_changetz_output(self):
        """Test timeline can parse timezone-aware changetz output"""
        result = self.run_qsv_command(
            f"load {self.get_fixture_path('simple.csv')} - changetz datetime --from-tz UTC --to-tz Asia/Tokyo - timeline datetime --interval 1h - show"
        )
        self.assertEqual(result.returncode, 0)
        self.assertEqual(result.stdout.strip(), "\n".join([
            "timeline_1h,count",
            "2023-01-01 21:00:00,1",
            "2023-01-01 22:00:00,1",
            "2023-01-01 23:00:00,1",
        ]))

if __name__ == "__main__":
    unittest.main()
