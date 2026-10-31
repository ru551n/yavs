-- VHDL-1993: `default` is an ordinary identifier here. VHDL-2008 reserved it.
package old_pkg is
  constant default : integer := 3;
  constant start   : integer := default;
end package;
