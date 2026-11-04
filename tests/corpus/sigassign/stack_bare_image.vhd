-- difftest: top=stack_bare_image ignore=order
-- ignore=order: several processes report in one delta cycle, in an order the LRM leaves open.
-- Wide buses nothing asks the history of next to record images: a plain bus as a field of a record
-- port (individually associated, both modes), a record with a wide field read whole, a wide field
-- assigned from a plain bus and back, a plain bus through a chain of ports read by level only.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

package stack_bare_p is
  type rec is record
    w : std_logic_vector(127 downto 0);
    b : std_logic;
    s : std_logic_vector(3 downto 0);
  end record;
  type wrec is record
    w : std_logic_vector(95 downto 0);
    n : natural range 0 to 1023;
  end record;
  function pop(a : std_logic_vector) return natural;
end package;

package body stack_bare_p is
  function pop(a : std_logic_vector) return natural is
    variable c : natural := 0;
  begin
    for i in a'range loop
      if a(i) = '1' then c := c + i + 1; end if;
    end loop;
    return c;
  end function;
end package body;

library ieee;
use ieee.std_logic_1164.all;
use work.stack_bare_p.all;

entity stack_bare_sink is
  generic (K : natural);
  port (clk : in std_logic; p : in rec; q : out rec; o : out natural);
end entity;

architecture a of stack_bare_sink is
  signal keep : rec := ((others => '0'), '0', "0000");
begin
  process (clk)
    variable x : rec;
    variable t : natural := 0;
    variable held : rec := ((others => '1'), '0', "1010");
  begin
    if rising_edge(clk) then
      x := p;
      t := (t + pop(x.w) + pop(x.s)) mod 100003;
      if x.b = '1' then t := t + K; end if;
      if x /= p then report "whole read differs" severity error; end if;
      keep <= x;
      if x.b = '1' then
        held.w := not x.w;
      end if;
      q <= held;
      o <= t;
    end if;
  end process;
end architecture;

library ieee;
use ieee.std_logic_1164.all;

entity stack_bare_chain is
  port (a : in std_logic_vector(99 downto 0); y : out std_logic_vector(99 downto 0));
end entity;

architecture a of stack_bare_chain is
begin
  y <= a;
end architecture;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use work.stack_bare_p.all;

entity stack_bare_image is
end entity;

architecture a of stack_bare_image is
  signal clk : std_logic := '0';
  signal n : natural := 0;
  signal bus_in : std_logic_vector(127 downto 0) := (others => '0');
  signal bit_in : std_logic := '0';
  signal nib : std_logic_vector(3 downto 0) := "0000";
  signal bus_out : std_logic_vector(127 downto 0);
  signal bit_out : std_logic;
  signal nib_out : std_logic_vector(3 downto 0);
  signal q2 : rec;
  signal o0, o1 : natural := 0;
  signal wr : wrec := ((others => '0'), 0);
  signal plain : std_logic_vector(95 downto 0) := (others => '0');
  signal back : std_logic_vector(95 downto 0);
  signal ch0, ch1, ch2 : std_logic_vector(99 downto 0) := (others => '0');
begin
  clk <= not clk after 5 ns when n < 400 else '0';

  i0 : entity work.stack_bare_sink generic map (1)
    port map (clk => clk,
              p.w => bus_in, p.b => bit_in, p.s => nib,
              q.w => bus_out, q.b => bit_out, q.s => nib_out,
              o => o0);
  i1 : entity work.stack_bare_sink generic map (2)
    port map (clk => clk, p.w => bus_out, p.b => bit_out, p.s => nib_out, q => q2, o => o1);
  c0 : entity work.stack_bare_chain port map (ch0, ch1);
  c1 : entity work.stack_bare_chain port map (ch1, ch2);

  -- A wide field from a plain bus, and a plain bus from the field.
  wr.w <= plain;
  back <= wr.w;

  stim : process (clk)
    variable lf : unsigned(127 downto 0) := x"0123456789abcdef_fedcba9876543210";
    variable x : wrec;
    variable t : natural := 0;
  begin
    if rising_edge(clk) then
      n <= n + 1;
      lf := lf(126 downto 0) & (lf(127) xor lf(125) xor lf(100) xor lf(98));
      -- Runs of holding and changing, so whole compare, whole copy and the image all act.
      if n mod 7 < 3 then
        bus_in <= std_logic_vector(lf);
      end if;
      if n mod 5 = 0 then
        bit_in <= not bit_in;
      end if;
      nib <= std_logic_vector(lf(3 downto 0));
      if n mod 3 /= 0 then
        plain <= std_logic_vector(lf(95 downto 0));
      end if;
      ch0 <= std_logic_vector(lf(99 downto 0));
      wr.n <= n mod 1024;
      x := wr;
      t := (t + pop(x.w) + x.n + pop(back) + pop(ch2)) mod 100003;
      if n mod 40 = 39 then
        report integer'image(t) & " " & integer'image(o0) & " " & integer'image(o1) & " "
          & to_hstring(q2.w) & " " & to_hstring(q2.s);
      end if;
    end if;
  end process;

  -- Level waiters only: no attribute, no edge function by name.
  level : process
  begin
    for i in 1 to 3 loop
      wait until ch2(17) = '1';
      report "ch2(17) high with bus " & to_hstring(ch2(31 downto 0));
      wait until bus_out(70) = '0';
      report "bus_out(70) low " & to_hstring(bus_out(31 downto 0));
    end loop;
    wait;
  end process;
end architecture;
