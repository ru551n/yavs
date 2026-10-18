-- rsim: --std 2019
package p is
  `warning "configured for simulation"
  `if TOOL_NAME = "rsim" then
  constant tool : string := "rsim";
  `elsif UNKNOWN_NAME = "x" then
  `end if
  `protect begin
  `if VHDL_VERSION = "2019" then
  `error "stop here"
  `end
end package;
