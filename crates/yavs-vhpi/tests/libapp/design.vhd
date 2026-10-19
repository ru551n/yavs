-- Library and application foreign models of tests/plugin/libapp.c (#357): `top` calls a
-- foreign function of library "used"; `none` uses no foreign model at all.
entity top is end entity;

architecture a of top is
  function one return integer;
  attribute foreign of one : function is "VHPI used one";
  signal s : integer := 0;
begin
  process
  begin
    wait for 5 ns;
    s <= one;
    wait for 1 ns;
    assert s = 1 report "one" severity failure;
    report "libapp ok";
    wait;
  end process;
end architecture;

entity none is end entity;

architecture a of none is
begin
  process
  begin
    wait for 5 ns;
    report "none ok";
    wait;
  end process;
end architecture;
