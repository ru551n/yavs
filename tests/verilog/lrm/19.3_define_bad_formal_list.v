// LRM 19.3: the formal arguments of a macro are simple identifiers separated by commas
// expect: E4008 4:15 empty formal argument
`timescale 1ns/1ps
`define BAD(a,, b) a
module m;
endmodule
