-- Byte codes: connections, sensitivity entries and signals passed to subprograms. Only `a` (an
-- element of it goes to a port the child reads whole), `d` (passed to a procedure that reads an
-- element of it), `f` (read by element in the process) and `g` (passed on twice to such a
-- procedure) keep their codes; `b` (an out port actual), `c` (a sensitivity entry), `e` (passed
-- to a procedure that reads it whole) and `h` (a port actual of a child that reads nothing) do
-- not. The report is the same with the codes of every signal (YAVS_CODES=all).
library ieee; use ieee.std_logic_1164.all;

entity rd_port is port (x : in std_logic; q : out std_logic); end;
architecture a of rd_port is begin
  process (x) begin q <= not x; end process;
end;

library ieee; use ieee.std_logic_1164.all;
entity wr_port is port (y : out std_logic; clk : in std_logic); end;
architecture a of wr_port is
  signal t : std_logic := '0';
begin
  process (clk) begin
    if rising_edge(clk) then t <= not t; end if;
  end process;
  y <= t;
end;

library ieee; use ieee.std_logic_1164.all;
entity no_port is port (z : in std_logic); end;
architecture a of no_port is begin end;

library ieee; use ieee.std_logic_1164.all;
entity elem_codes_conn is end;
architecture a of elem_codes_conn is
  signal clk : std_logic := '0';
  signal n : natural := 0;
  signal a, b, c, d, e, f, g, h : std_logic_vector(15 downto 0) := x"A5C3";
  signal qa : std_logic;
  signal hits : natural := 0;
  signal done : boolean := false;

  procedure bit_of (signal s : in std_logic_vector; variable r : out std_logic) is
  begin r := s(3); end;
  procedure pass_on (signal s : in std_logic_vector; variable r : out std_logic) is
  begin bit_of(s, r); end;
  procedure all_of (signal s : in std_logic_vector; variable r : out std_logic) is
  begin r := '1' when s = x"0000" else '0'; end;
begin
  clk <= not clk after 5 ns when not done;
  u1: entity work.rd_port port map (x => a(3), q => qa);
  u2: entity work.wr_port port map (y => b(5), clk => clk);
  u3: entity work.no_port port map (z => h(2));
  process (c(7), clk) begin
    if rising_edge(clk) then
      n <= n + 1;
      a <= a(14 downto 0) & a(15 downto 15);
      c <= c(14 downto 0) & '1';
      d <= d(14 downto 0) & d(15 downto 15);
      e <= e(14 downto 0) & e(15 downto 15);
      f <= f(14 downto 0) & f(2);
      g <= g(14 downto 0) & g(15 downto 15);
      h <= h(14 downto 0) & h(15 downto 15);
    end if;
  end process;
  process (clk)
    variable v : std_logic;
  begin
    if rising_edge(clk) then
      bit_of(d, v);
      if v = '1' then hits <= hits + 1; end if;
      all_of(e, v);
      pass_on(g, v);
      if v = '1' then hits <= hits + 2; end if;
    end if;
  end process;
  process
  begin
    wait until n = 300;
    report "a=" & to_string(a) & " b=" & to_string(b) & " c=" & to_string(c) & " d=" & to_string(d)
      & " e=" & to_string(e) & " f=" & to_string(f) & " g=" & to_string(g) & " h=" & to_string(h)
      & " qa=" & std_logic'image(qa) & " hits=" & integer'image(hits);
    done <= true; wait;
  end process;
end;
