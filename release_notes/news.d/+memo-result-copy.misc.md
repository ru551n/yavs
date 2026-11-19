A call whose array result the memo of pure calls keeps now copies the result once, straight into the memo's own storage, so a run of calls that never repeat does less work per call.
