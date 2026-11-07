// LRM 4.4.2: a drive strength goes with a net declaration assignment
// expect: E4100 4:8 drive strength without assignment
module m;
  wire (strong0, strong1) w;
endmodule
