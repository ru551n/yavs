-- A whole record drive compared with the record's image: at three clocks of four the whole value
-- holds, at the fourth one scalar and the leaves next to it do not. `scatter_spared` counts the
-- leaves the compare spared, and the image pays at the review after a thousand scatters.
library ieee; use ieee.std_logic_1164.all; use ieee.numeric_std.all;
entity rec_cmp is generic (CYC : natural := 1200); end;
architecture a of rec_cmp is
  type rec is record
    a : std_ulogic;
    b : std_ulogic_vector(15 downto 0);
    i : integer;
    c : std_ulogic;
    d : std_ulogic_vector(31 downto 0);
    n : unsigned(7 downto 0);
    e : std_ulogic;
  end record;
  signal clk : std_ulogic := '0';
  signal n : natural := 0;
  signal r : rec := ('0', x"1234", 7, '1', x"89abcdef", x"00", '0');
begin
  reg : process (clk)
    variable v : rec := ('0', x"1234", 7, '1', x"89abcdef", x"00", '0');
  begin
    if rising_edge(clk) then
      n <= n + 1;
      if n mod 4 = 0 then
        v.n := v.n + 1;
      end if;
      r <= v;
    end if;
  end process;
  main : process
  begin
    for i in 1 to CYC loop
      clk <= '1'; wait for 5 ns;
      clk <= '0'; wait for 5 ns;
    end loop;
    report "n=" & integer'image(to_integer(r.n)) & " i=" & integer'image(r.i);
    wait;
  end process;
end;
