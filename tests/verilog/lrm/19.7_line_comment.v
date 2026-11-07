// LRM 19.7: no comment may follow `line on its line
// expect: E4009 3:1 comment after line
`line 5 "f.v" 0 // note
module m;
endmodule
