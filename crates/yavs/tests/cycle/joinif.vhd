-- Cycle mode: consecutive `if`s whose conditions are computed the same way join behind one branch
-- (`join_ifs`): a register file written bit by bit under `if idx = k`, and variables updated in
-- place before each `if` (`a := a and en`), whose conditions look alike but read other values.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity cycle_joinif is
  generic (W : positive := 8);
end entity;
architecture a of cycle_joinif is
  type regs_t is array (0 to 3) of std_ulogic_vector(W - 1 downto 0);
  signal clk : std_ulogic := '0';
  signal idx : unsigned(1 downto 0) := (others => '0');
  signal we, p, q, en : std_ulogic := '0';
  signal d : std_ulogic_vector(W - 1 downto 0) := (others => '0');
  signal regs : regs_t := (others => (others => '0'));
  signal x, y, z : std_ulogic;
  signal rd : std_ulogic_vector(W - 1 downto 0);
begin
  clk <= not clk after 5 ns;

  write : process (clk)
  begin
    if rising_edge(clk) then
      for k in regs'range loop
        for b in 0 to W - 1 loop
          if idx = k then
            if we = '1' then
              regs(k)(b) <= d(b);
            end if;
          end if;
        end loop;
      end loop;
    end if;
  end process;

  read : process (all)
  begin
    rd <= (others => '0');
    for k in regs'range loop
      for b in 0 to W - 1 loop
        if idx = k then
          rd(b) <= regs(k)(b);
        end if;
      end loop;
    end loop;
  end process;

  vars : process (all)
    variable a, b, c : std_ulogic;
  begin
    a := p;
    b := q;
    c := p;
    x <= '0';
    y <= '0';
    z <= '0';
    a := a and en;
    if a = '1' then
      x <= '1';
    end if;
    b := b and en;
    if b = '1' then
      y <= '1';
    end if;
    c := c and en;
    if c = '1' then
      z <= '1';
      c := '0';
    end if;
    if c = '1' then
      z <= '0';
    end if;
  end process;

  stim : process
    variable n : natural := 0;
  begin
    for i in 0 to 23 loop
      wait until falling_edge(clk);
      report "rd=" & to_hstring(rd) & " r0=" & to_hstring(regs(0)) & " r3=" & to_hstring(regs(3))
        & " x=" & std_ulogic'image(x) & " y=" & std_ulogic'image(y) & " z=" & std_ulogic'image(z);
      n := (n * 5 + 3) mod 16;
      idx <= to_unsigned(n mod 4, 2);
      we <= '1' when n mod 3 /= 0 else '0';
      d <= not d(W - 2 downto 0) & d(W - 1) when W > 1 else not d;
      p <= '1' when n mod 2 = 1 else '0';
      q <= '1' when n mod 4 = 2 else '0';
      en <= '1' when n mod 5 /= 1 else '0';
    end loop;
    std.env.finish;
  end process;
end architecture;
