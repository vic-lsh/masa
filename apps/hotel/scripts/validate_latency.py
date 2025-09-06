import csv
import argparse
import sys

def validate_latency_chain(file_path):
    """
    Reads a CSV and asserts that for every row:
    child_reserve_e2e_latency_frontend >= child_reserve_e2e_latency >= child_reserve_e2e_latency_in_handler
    """
    try:
        with open(file_path, mode='r', newline='', encoding='utf-8') as csvfile:
            # Use DictReader to easily access columns by name
            reader = csv.DictReader(csvfile)
            
            # Define the specific columns we need to check
            required_cols = [
                'child_reserve_e2e_latency_frontend',
                'child_reserve_e2e_latency',
                'child_reserve_e2e_latency_in_handler'
            ]

            # Verify that the required columns exist in the CSV header
            if not all(col in reader.fieldnames for col in required_cols):
                missing = set(required_cols) - set(reader.fieldnames)
                sys.exit(f"❌ Error: CSV is missing required columns: {', '.join(missing)}")

            for row_num, row in enumerate(reader, start=2):
                try:
                    if row['error'] != "/None":
                        continue
                    # Extract and convert the relevant latency values to float
                    frontend_latency = float(row['child_reserve_e2e_latency_frontend'])
                    e2e_latency = float(row['child_reserve_e2e_latency'])
                    handler_latency = float(row['child_reserve_e2e_latency_in_handler'])
                    
                    # The main assertion logic
                    assert frontend_latency >= e2e_latency >= handler_latency

                except ValueError:
                    print(f"⚠️  Warning: Non-numeric data found in row {row_num}. Skipping assertion.")
                    continue
                except AssertionError:
                    # If the assertion fails, print a detailed error and exit
                    print(f"❌ Assertion failed at row {row_num}:")
                    print(f"   Condition not met: {frontend_latency} >= {e2e_latency} >= {handler_latency}")
                    sys.exit(1) # Exit with a non-zero status code to indicate failure
                except Exception as e:
                    sys.exit(f"❌ An unexpected error occurred: {e}, row {row}")

        print("✅ All rows passed validation successfully!")

    except FileNotFoundError:
        sys.exit(f"❌ Error: The file at path '{file_path}' was not found.")
    except Exception as e:
        sys.exit(f"❌ An unexpected error occurred: {e}")

if __name__ == "__main__":
    # Set up the command-line argument parser
    parser = argparse.ArgumentParser(
        description="Validate latency values in a specified CSV file."
    )
    parser.add_argument(
        "csv_path",
        type=str,
        help="The file path to the CSV to be validated."
    )
    
    args = parser.parse_args()
    
    # Call the validation function with the provided file path
    validate_latency_chain(args.csv_path)