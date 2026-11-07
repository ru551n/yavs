// LRM 19.4: `elsif is followed by a macro name
// expect: E4009 4:1 missing name
`ifdef A
`elsif
`endif
module m;
endmodule
