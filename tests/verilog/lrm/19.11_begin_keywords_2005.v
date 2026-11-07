// LRM 19.11: under "1364-2005" uwire is a keyword and cannot name a net
// expect: E4100 5:8 uwire is reserved
`begin_keywords "1364-2005"
module m;
  wire uwire;
endmodule
`end_keywords
