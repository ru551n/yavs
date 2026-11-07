// LRM 12.3: the port list has no trailing comma
// expect: E4100 4:20 port expected after the comma
`timescale 1ns/1ps
module m (input a, );
endmodule
