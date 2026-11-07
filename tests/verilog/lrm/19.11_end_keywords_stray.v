// LRM 19.11: `end_keywords closes a `begin_keywords
// expect: E4004 5:1 nothing to close
module m;
endmodule
`end_keywords
