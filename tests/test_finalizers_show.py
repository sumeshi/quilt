import unittest
from test_base import QsvTestBase

class TestShow(QsvTestBase):
    
    def test_show_basic_csv_output(self):
        """Test show command outputs CSV format with headers"""
        result = self.run_qsv_command(f"load {self.get_fixture_path('simple.csv')} - show")
        self.assertEqual(result.returncode, 0)
        
        expected_output = '\n'.join([
            "datetime,col1,col2,col3,str",
            "2023-01-01 12:00:00,1,2,3,foo",
            "2023-01-01 13:00:00,4,5,6,bar",
            "2023-01-01 14:00:00,7,8,9,baz",
        ])
        self.assertEqual(result.stdout.strip(), expected_output)
    
    def test_show_with_select(self):
        """Test show command with select operation"""
        result = self.run_qsv_command(f"load {self.get_fixture_path('simple.csv')} - select col1,str - show")
        self.assertEqual(result.returncode, 0)
        
        expected_output = '\n'.join([
            "col1,str",
            "1,foo",
            "4,bar",
            "7,baz",
        ])
        self.assertEqual(result.stdout.strip(), expected_output)
    
    def test_show_with_head(self):
        """Test show command with head operation"""
        result = self.run_qsv_command(f"load {self.get_fixture_path('simple.csv')} - head 2 - show")
        self.assertEqual(result.returncode, 0)
        
        expected_output = '\n'.join([
            "datetime,col1,col2,col3,str",
            "2023-01-01 12:00:00,1,2,3,foo",
            "2023-01-01 13:00:00,4,5,6,bar",
        ])
        self.assertEqual(result.stdout.strip(), expected_output)
    
    def test_show_with_sort(self):
        """Test show command with sort operation"""
        result = self.run_qsv_command(f"load {self.get_fixture_path('simple.csv')} - sort str --desc - show")
        self.assertEqual(result.returncode, 0)
        
        # When sorted by str column in descending order: foo, baz, bar
        expected_output = '\n'.join([
            "datetime,col1,col2,col3,str",
            "2023-01-01 12:00:00,1,2,3,foo",
            "2023-01-01 14:00:00,7,8,9,baz",
            "2023-01-01 13:00:00,4,5,6,bar",
        ])
        self.assertEqual(result.stdout.strip(), expected_output)
    
    def test_show_with_filter(self):
        """Test show command with isin filter"""
        result = self.run_qsv_command(f"load {self.get_fixture_path('simple.csv')} - isin col1 1,7 - show")
        self.assertEqual(result.returncode, 0)
        
        expected_output = '\n'.join([
            "datetime,col1,col2,col3,str",
            "2023-01-01 12:00:00,1,2,3,foo",
            "2023-01-01 14:00:00,7,8,9,baz",
        ])
        self.assertEqual(result.stdout.strip(), expected_output)
    
    def test_show_empty_dataframe(self):
        """Test show command with empty result"""
        result = self.run_qsv_command(f"load {self.get_fixture_path('simple.csv')} - isin col1 999 - show")
        self.assertEqual(result.returncode, 0)
        
        # Should only show headers when no data matches
        expected_output = "datetime,col1,col2,col3,str"
        self.assertEqual(result.stdout.strip(), expected_output)
    
    def test_show_with_multiple_operations(self):
        """Test show command with multiple chained operations"""
        result = self.run_qsv_command(f"load {self.get_fixture_path('simple.csv')} - select datetime,col1,str - isin str foo,bar - sort col1 - show")
        self.assertEqual(result.returncode, 0)
        
        expected_output = '\n'.join([
            "datetime,col1,str",
            "2023-01-01 12:00:00,1,foo",
            "2023-01-01 13:00:00,4,bar",
        ])
        self.assertEqual(result.stdout.strip(), expected_output)

    def test_show_does_not_add_extra_blank_line(self):
        """Test non-streaming show output ends with a single newline"""
        result = self.run_qsv_command(f"load {self.get_fixture_path('simple.csv')} - show")
        self.assertEqual(result.returncode, 0)
        self.assertTrue(result.stdout.endswith("\n"))
        self.assertFalse(result.stdout.endswith("\n\n"))

if __name__ == "__main__":
    unittest.main()
