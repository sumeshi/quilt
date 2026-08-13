#!/usr/bin/env python3
import unittest
import sys
import os

from test_command_surface import TestCommandSurface

# Initializers
from test_initializers_load import TestLoad

# Chainables
from test_chainables_select import TestSelect
from test_chainables_cast import TestCast
from test_chainables_bucket import TestBucket
from test_chainables_delta import TestDelta
from test_chainables_extract import TestExtract
from test_chainables_flatten import TestFlatten
from test_chainables_parse_size import TestParseSize
from test_chainables_head import TestHead
from test_chainables_tail import TestTail
from test_chainables_contains import TestContains
from test_chainables_grep import TestGrep
from test_chainables_changetz import TestChangetz
from test_chainables_isin import TestIsin
from test_chainables_sed import TestSed
from test_chainables_sort import TestSort
from test_chainables_count import TestCount
from test_chainables_uniq import TestUniq
from test_chainables_renamecol import TestRenamecol
from test_chainables_timeslice import TestTimeslice

# Finalizers
from test_finalizers_headers import TestHeaders
from test_finalizers_calc import TestCalc
from test_finalizers_dump import TestDump
from test_finalizers_stats import TestStats
from test_finalizers_partition import TestPartition
from test_finalizers_showquery import TestShowquery
from test_finalizers_showtable import TestShowtable
from test_finalizers_show import TestShow

# Quilters
from test_quilters_quilt import TestQuilt

def run_test_suite():
    loader = unittest.TestLoader()
    suite = unittest.TestSuite()
    
    # Initializers
    initializers = [
        TestLoad,
    ]
    for initializer in initializers:
        suite.addTest(loader.loadTestsFromTestCase(initializer))
    
    # Chainables
    chainables = [
        TestSelect,
        TestCast,
        TestBucket,
        TestDelta,
        TestExtract,
        TestFlatten,
        TestParseSize,
        TestHead,
        TestTail,
        TestContains,
        TestGrep,
        TestChangetz,
        TestIsin,
        TestSed,
        TestSort,
        TestCount,
        TestUniq,
        TestRenamecol,
        TestTimeslice,
    ]
    for chainable in chainables:
        suite.addTest(loader.loadTestsFromTestCase(chainable))

    suite.addTest(loader.loadTestsFromTestCase(TestCommandSurface))
    
    # Finalizers
    finalizers = [
        TestCalc,
        TestHeaders,
        TestDump,
        TestStats,
        TestPartition,
        TestShowquery,
        TestShowtable,
        TestShow,
    ]
    for finalizer in finalizers:
        suite.addTest(loader.loadTestsFromTestCase(finalizer))
    
    # Quilters
    quilters = [
        TestQuilt,
    ]
    for quilter in quilters:
        suite.addTest(loader.loadTestsFromTestCase(quilter))
    
    # Run the tests
    print("\nRunning tests...")
    runner = unittest.TextTestRunner()
    result = runner.run(suite)
    
    # Print summary
    print(f"\n{'='*60}")
    print("TEST SUMMARY")
    print(f"{'='*60}")
    print(f"Tests run: {result.testsRun}")
    print(f"Failures: {len(result.failures)}")
    print(f"Errors: {len(result.errors)}")
    print(f"Skipped: {len(result.skipped) if hasattr(result, 'skipped') else 0}")
    
    if result.failures:
        print(f"\nFAILURES ({len(result.failures)}):")
        for test, traceback in result.failures:
            print(f"  - {test}")
    
    if result.errors:
        print(f"\nERRORS ({len(result.errors)}):")
        for test, traceback in result.errors:
            print(f"  - {test}")
    
    success_rate = (result.testsRun - len(result.failures) - len(result.errors)) / result.testsRun * 100 if result.testsRun > 0 else 0
    print(f"\nSuccess rate: {success_rate:.1f}%")
    
    return result.wasSuccessful()

if __name__ == "__main__":
    success = run_test_suite()
    sys.exit(0 if success else 1)
