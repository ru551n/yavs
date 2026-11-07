// LRM 19.11: `begin_keywords and `end_keywords select a keyword set for the text between them
// units: m
// expect: none
`timescale 1ns/1ps
`begin_keywords "1364-2005"
module m;
endmodule
`end_keywords
