-- A check that fails only on the second image of the metavalue contract (no --two-state) (A38): `idx` of a register
-- still 'U' (0 in the region, all ones in the second image), copied to a variable first (a
-- `to_integer` of a signal gives 0 there, as the normal mode, A43), is out of its range there.
-- With MID false it fails at initialisation, with MID true at the clock edge where `go` rises,
-- after 'U' of `r` and `q` was driven.
library ieee; use ieee.std_logic_1164.all; use ieee.numeric_std.all;
entity cycle_railchk is
  generic (W : positive := 8; MID : boolean := true);
end;
architecture a of cycle_railchk is
  signal clk, go, en : std_ulogic := '0';
  signal r, q : std_ulogic_vector(W - 1 downto 0);
  signal idx : integer range 0 to 0 := 0;
begin
  clk <= not clk after 5 ns when now < 200 ns else clk;
  reg : process (clk) begin
    if rising_edge(clk) then
      if en = '1' then
        r <= (others => '0');
      end if;
      q <= r;
    end if;
  end process;
  ix : process (go, r)
    variable v : std_ulogic_vector(0 downto 0);
  begin
    v := r(W - 1 downto W - 1);
    if go = '1' or not MID then
      idx <= to_integer(unsigned(v));
    else
      idx <= 0;
    end if;
  end process;
  stim : process begin
    wait for 12 ns; report "a r=" & to_string(r) & " q=" & to_string(q) & " idx=" & integer'image(idx);
    go <= '1';
    wait for 10 ns; report "b r=" & to_string(r) & " q=" & to_string(q) & " idx=" & integer'image(idx);
    en <= '1';
    wait for 20 ns; report "c r=" & to_string(r) & " q=" & to_string(q) & " idx=" & integer'image(idx);
    wait;
  end process;
end;
