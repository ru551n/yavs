-- A combinational process on a record register (a record pack): its sensitivity names every
-- field, which the kernel joins into one waiter for the whole signal; that process is woken
-- where the register's update finishes, without a scan of waiter lists.
library ieee; use ieee.std_logic_1164.all; use ieee.numeric_std.all;
entity rec_wake is generic (CYC : natural := 200); end;
architecture a of rec_wake is
  type rec is record
    a : std_ulogic;
    b : std_ulogic_vector(15 downto 0);
    c : std_ulogic;
    d : std_ulogic_vector(31 downto 0);
    e : std_ulogic;
    n : unsigned(15 downto 0);
  end record;
  signal clk : std_ulogic := '0';
  signal r, rin : rec := ('0', x"1234", '1', x"89abcdef", '0', (others => '0'));
begin
  comb : process (r)
    variable v : rec;
  begin
    v := r;
    v.n := v.n + 1;
    v.a := v.c xor v.e;
    v.b := v.b(14 downto 0) & (v.b(15) xor v.d(0));
    v.c := v.a;
    v.d := v.d(30 downto 0) & (v.d(31) xor v.b(3));
    v.e := v.n(0);
    rin <= v;
  end process;
  reg : process (clk)
  begin
    if rising_edge(clk) then
      r <= rin;
    end if;
  end process;
  main : process
  begin
    for i in 1 to CYC loop
      clk <= '1'; wait for 5 ns;
      clk <= '0'; wait for 5 ns;
    end loop;
    report "n=" & integer'image(to_integer(r.n)) & " b=" & to_hstring(r.b) & " d=" & to_hstring(r.d);
    wait;
  end process;
end;
