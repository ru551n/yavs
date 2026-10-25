Interpreted code runs fewer statement-end operations: a statement that has nothing to free no longer pays for one, which speeds up call-heavy and file and text I/O code.
