-- difftest: top=rtl_vector_ops std=2008 stop=2us
-- Clocked processes that read, concatenate and drive logic vectors of several widths and
-- offsets with every std_ulogic value: the paths a compiled tier takes inline.
library ieee;
use ieee.std_logic_1164.all;

entity rtl_vector_ops is end;

architecture t of rtl_vector_ops is
  signal clk : std_logic := '0';
  signal a   : std_logic_vector(15 downto 0) := "UX01ZWLH-UX01ZWL";
  signal b   : std_logic_vector(63 downto 0) := (others => '0');
  signal c   : std_logic_vector(64 downto 0) := (others => '1');
  signal d   : std_logic_vector(2 downto 0) := "01Z";
  signal e   : std_logic_vector(0 downto 0) := "0";
  signal w   : std_logic_vector(31 downto 0) := (others => 'Z');
  type arr is array (0 to 3) of std_logic_vector(7 downto 0);
  signal r   : arr := (x"A5", "UUUUUUUU", "XXXXXXXX", "ZZZZZZZZ");
  signal n   : natural := 0;
begin
  clk <= not clk after 5 ns when n < 40 else clk;

  -- a rotates and takes a new bit from an xor of four taps (scalar natives on all nine codes)
  p1 : process (clk)
  begin
    if rising_edge(clk) then
      a <= a(14 downto 0) & (a(15) xor a(13) xor a(12) xor a(10));
    end if;
  end process;

  -- a 64-bit shift register with a constant fill and a slice read at an odd offset
  p2 : process (clk)
  begin
    if rising_edge(clk) then
      b <= b(62 downto 0) & not b(63);
    end if;
  end process;

  -- 65 elements: past one block
  p3 : process (clk)
  begin
    if rising_edge(clk) then
      c <= c(63 downto 0) & c(64);
    end if;
  end process;

  -- the same value assigned again, a 1-element vector and a 3-element vector with a code
  -- that is not 0 or 1
  p4 : process (clk)
  begin
    if rising_edge(clk) then
      d <= d(1 downto 0) & d(2);
      e <= not e;
    end if;
  end process;

  -- unaligned slice of a wider vector driven by two processes on disjoint halves
  p5 : process (clk)
  begin
    if rising_edge(clk) then
      w(15 downto 0) <= a;
    end if;
  end process;
  p6 : process (clk)
  begin
    if rising_edge(clk) then
      w(31 downto 16) <= b(40 downto 25);
    end if;
  end process;

  -- an array of vectors: one element per process-visible index
  p7 : process (clk)
  begin
    if rising_edge(clk) then
      r(n mod 4) <= r((n + 1) mod 4)(6 downto 0) & r((n + 3) mod 4)(7);
    end if;
  end process;

  p8 : process (clk)
  begin
    if rising_edge(clk) then
      n <= n + 1;
    end if;
  end process;

  report_p : process (clk)
  begin
    if falling_edge(clk) then
      report to_string(a) & " " & to_string(b) & " " & to_string(c) & " " & to_string(d)
        & " " & to_string(e) & " " & to_string(w) & " " & to_string(r(0)) & " "
        & to_string(r(1)) & " " & to_string(r(2)) & " " & to_string(r(3));
    end if;
  end process;
end;
