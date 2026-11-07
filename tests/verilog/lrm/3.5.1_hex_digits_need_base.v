// LRM 3.5.1: 4af is illegal: hexadecimal digits need the 'h base
// expect: E4014 4:17 malformed number
module m;
  parameter p = 4af;
endmodule
