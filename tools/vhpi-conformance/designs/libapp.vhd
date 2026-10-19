-- Library and application foreign models (plugins/libapp.c). The design calls a foreign
-- function of the same library, which also makes one reference simulator load the plugin
-- (the library word is the plugin's file name). The design never names the library or
-- application models themselves.
entity libapp is end entity;

architecture sim of libapp is
  function f return integer;
  attribute foreign of f : function is "VHPI libapp f";
  function f return integer is begin return 0; end function;
  signal s : integer := -1;
begin
  process
  begin
    wait for 1 ns;
    s <= f;
    wait for 1 ns;
    assert s = 7 report "f" severity note;
    wait;
  end process;
end architecture;
