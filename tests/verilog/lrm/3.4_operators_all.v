// LRM 3.4: every 1364-2005 operator is a token
// units: m
// expect: none
`timescale 1ns/1ps
module m;
  integer a, b, c;
  reg [3:0] v;
  initial begin
    c = a + b - a * b / a % b ** 2;
    c = (a < b) | (a > b) | (a <= b) | (a >= b) | (a == b) | (a != b) | (a === b) | (a !== b);
    c = (a && b) || !a;
    c = ~a & b | a ^ b ~^ a ^~ b;
    c = &v | ~&v | |v | ~|v | ^v | ~^v | ^~v;
    c = a << 1 >> 1;
    c = a <<< 1 >>> 1;
    c = a ? b : c;
    c = {a, b} + {2{a}};
  end
endmodule
