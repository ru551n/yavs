// LRM 19.5: only white space or a comment may follow `include on its line
// yavs-test: args=-I inc
// expect: E4009 5:1 text after the name
module m;
`include "net.vh" wire w;
endmodule
