-- difftest: top=sig_array_part std=2008
-- Reading an element, a slice of an element and one scalar of an element of an array of
-- arrays signal, with constant and run-time indices (#436).
entity sig_array_part is
end entity;

architecture a of sig_array_part is
  type vec4 is array (natural range <>) of bit_vector(7 downto 0);
  type vec2 is array (0 to 1, 0 to 1) of bit_vector(3 downto 0);
  signal r  : vec4(1 to 3) := (x"A5", x"3C", x"0F");
  signal up : vec4(0 to 1) := (x"01", x"80");
  signal m  : vec2 := (("0001", "0010"), ("0100", "1000"));
begin
  process
    variable k : integer := 2;
    variable e : bit_vector(7 downto 0);
    variable n : bit_vector(3 downto 0);
  begin
    wait for 1 ns;
    assert r(2) = x"3C" report "r(2)" severity failure;
    assert r(k) = x"3C" report "r(k)" severity failure;
    assert r(3)(3 downto 0) = x"F" report "slice const" severity failure;
    assert r(k + 1)(7 downto 4) = x"0" report "slice dyn" severity failure;
    assert r(1)(7) = '1' and r(1)(6) = '0' report "scalar const" severity failure;
    assert r(k)(k) = '1' and r(k)(k + 1) = '1' report "scalar dyn" severity failure;
    assert up(1)(7) = '1' and up(0)(0) = '1' report "ascending" severity failure;
    assert up(0)(7 downto 4) = x"0" report "up slice" severity failure;
    assert m(1, 0) = "0100" and m(0, 1)(1) = '1' report "2d" severity failure;
    assert m(1, 1)(3 downto 2) = "10" report "2d slice" severity failure;
    r(2) <= r(1)(3 downto 0) & r(3)(7 downto 4);
    up(0)(3 downto 0) <= r(1)(7 downto 4);
    wait for 1 ns;
    e := r(2);
    n := up(0)(3 downto 0);
    assert e = x"50" report "assign" severity failure;
    assert n = x"A" report "assign slice" severity failure;
    report "sig_array_part done";
    wait;
  end process;
end architecture;
