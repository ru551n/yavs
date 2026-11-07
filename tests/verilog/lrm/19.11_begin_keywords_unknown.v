// LRM 19.11: an unrecognised version specifier of `begin_keywords is an error
// expect: E4009 3:1 unknown version
`begin_keywords "1364-1999"
module m;
endmodule
`end_keywords
