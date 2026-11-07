// LRM 19.8: a module with no `timescale in force gets the tool default, and yavs says so (19.8 makes modules with and without a timescale in one design an error: known gap G9, W4006 is only a warning)
// expect: W4006 3:8 no timescale
module m;
endmodule
