-- difftest: top=std_logic_1164_ops std=2008
-- std_logic_1164 body: resolution, logic tables, conversions, shifts, edges, strings, matching.
library ieee; use ieee.std_logic_1164.all;
entity std_logic_1164_ops is end;
architecture t of std_logic_1164_ops is
  signal r : std_logic;
  signal clk : std_ulogic := '0';
  signal rises : integer := 0;
begin
  r <= '1'; r <= 'Z';
  process (clk) begin
    if rising_edge(clk) then rises <= rises + 1; end if;
  end process;
  process
    variable v : std_logic_vector(7 downto 0) := "1010ZX0U";
    variable a : std_logic_vector(3 downto 0) := "1100";
    variable b : std_logic_vector(3 downto 0) := "1010";
    variable bv : bit_vector(3 downto 0);
  begin
    wait for 1 ns;
    report "resolved " & std_logic'image(r);
    assert r = '1' severity failure;
    report "and " & to_string(a and b) & " or " & to_string(a or b) & " xor " & to_string(a xor b);
    report "nand " & to_string(a nand b) & " nor " & to_string(a nor b) & " xnor " & to_string(a xnor b);
    report "not " & to_string(not v);
    assert (a and b) = "1000" and (a xor b) = "0110" and (not a) = "0011" severity failure;
    assert ('1' and 'X') = 'X' and ('0' and 'X') = '0' and ('1' or 'U') = '1' severity failure;
    report "reduce and=" & std_ulogic'image(and a) & " or=" & std_ulogic'image(or a) & " xor=" & std_ulogic'image(xor b);
    report "x01 " & to_string(to_x01(v)) & " x01z " & to_string(to_x01z(v)) & " ux01 " & to_string(to_ux01(v));
    assert to_x01(v) = "1010XX0X" severity failure;
    assert is_x(v) and not is_x(a) severity failure;
    bv := to_bitvector(a);
    report "bv " & to_string(bv) & " slv " & to_string(to_stdlogicvector(bv));
    assert to_bitvector(std_logic_vector'("1X0Z"), '1') = "1101" severity failure;
    report "sll " & to_string(a sll 1) & " srl " & to_string(a srl 2) & " rol " & to_string(a rol 1) & " ror " & to_string(a ror 3);
    report "hstring " & to_hstring(std_logic_vector'(x"A5")) & " ostring " & to_ostring(std_logic_vector'("101110"));
    assert to_hstring(std_logic_vector'("1Z01")) = "X" severity failure;
    report "match " & std_ulogic'image('1' ?= 'H') & " " & std_ulogic'image(a ?= "11--") & " " & std_ulogic'image('0' ?< '1');
    assert ("1-0" ?= std_logic_vector'("110")) = '1' severity failure;
    for i in 1 to 4 loop clk <= '1'; wait for 1 ns; clk <= '0'; wait for 1 ns; end loop;
    report "rises " & integer'image(rises);
    assert rises = 4 severity failure;
    report "done";
    wait;
  end process;
end;
