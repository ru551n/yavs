// LRM 4.10.1: a parameter has a type or a range, not both
// expect: E4100 4:21 integer with range
module m;
  parameter integer [3:0] p = 1;
endmodule
