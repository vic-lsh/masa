# AGENTS.md

## Rules for editing code

- Be sure to update the README if the how to use the code changes.
- Consider whether your code edit has:
  - made other code redundant (e.g., unused now). Delete dead code if found.
  - made the code more complex (e.g., more code to maintain). Consider how you can simplify the design if this new feature to implement is part of the requirements in the first place.
- When introducing a new feature, write tests for it.
- When updating existing code's behavior, either update the test to reflect the new behavior, or add a new test to cover the new behavior.

### Code format

- Run formatting script to format the code.

## Codebase detail

### exp.runner

- When updating the runner, make sure to update the READMEs if the CLI interface changes.
- When updating the runner, make sure to add/update/remove test cases if necessary.