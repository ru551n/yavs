-- difftest: top=rtl_process_variables std=2008 stop=1us
-- Clocked processes whose array variables live in the heap: read, shifted and driven every
-- cycle, next to a process that allocates and frees an access object each cycle (the host
-- addresses a compiled tier keeps between activations must not outlive the object).
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

entity rtl_process_variables is end;

architecture t of rtl_process_variables is
  signal clk : std_logic := '0';
  signal n   : natural := 0;
  type word_arr is array (0 to 3) of unsigned(15 downto 0);
  signal q : word_arr;
  signal s : std_logic_vector(15 downto 0) := (others => '0');
  signal t : std_logic_vector(7 downto 0) := (others => 'U');
begin
  clk <= not clk after 5 ns when n < 60 else clk;

  g : for i in 0 to 3 generate
    p : process (clk)
      variable v : unsigned(15 downto 0) := to_unsigned(i * 37 + 1, 16);
    begin
      if rising_edge(clk) then
        v := v(14 downto 0) & (v(15) xor v(13) xor v(12) xor v(10));
        q(i) <= v;
      end if;
    end process;
  end generate;

  pa : process (clk)
    type ptr is access std_logic_vector;
    variable p    : ptr;
    variable keep : std_logic_vector(15 downto 0) := x"BEEF";
    variable cnt  : natural := 0;
  begin
    if rising_edge(clk) then
      p := new std_logic_vector'(keep);
      keep := keep(0) & keep(15 downto 1);
      p.all(3 downto 0) := keep(7 downto 4);
      s <= p.all xor keep;
      if cnt mod 3 = 0 then
        deallocate(p);
      else
        p.all(7 downto 0) := p.all(15 downto 8);
        t <= p.all(7 downto 0);
        deallocate(p);
      end if;
      cnt := cnt + 1;
    end if;
  end process;

  rep : process (clk)
  begin
    if rising_edge(clk) then
      n <= n + 1;
      report to_string(q(0)) & " " & to_string(q(1)) & " " & to_string(q(2)) & " "
           & to_string(q(3)) & " " & to_string(s) & " " & to_string(t);
    end if;
  end process;
end architecture;
