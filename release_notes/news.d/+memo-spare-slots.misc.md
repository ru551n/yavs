A call whose result the memo of pure calls keeps now reuses the memory of a result it dropped, so a run of calls that never repeat allocates less.
