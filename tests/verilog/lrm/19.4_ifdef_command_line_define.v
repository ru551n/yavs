// LRM 19.4: a macro defined on the command line is defined in the first file
// yavs-test: args=-D FROM_CLI
// units: a
// expect: none
`timescale 1ns/1ps
`ifdef FROM_CLI
module a;
endmodule
`endif
