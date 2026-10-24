library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

-- The carry-in idiom `(0 to 0 => cin)` is straight code (no loop in the process), a slice of a
-- signal is read as a slice (no copy out of the whole signal) and the signal copies through
-- `std_logic_vector(...)` / `unsigned(...)` run in the kernel: the ops of the interpreter are
-- those of the adder process and the stimulus only.
entity agg_carry is
end entity;

architecture t of agg_carry is
  signal a, b : std_logic_vector(7 downto 0) := (others => '0');
  signal cin  : std_logic := '0';
  signal z    : unsigned(7 downto 0);
  signal s    : std_logic_vector(6 downto 0);
  signal ss   : unsigned(6 downto 0);
  signal hi   : std_logic_vector(2 downto 0);
begin
  z  <= ('0' & unsigned(a(6 downto 0))) + unsigned(b(6 downto 0)) + (0 to 0 => cin);
  s  <= std_logic_vector(z(6 downto 0));
  ss <= unsigned(s);
  hi <= std_logic_vector(ss(6 downto 4));

  process
    variable acc : natural := 0;
  begin
    for i in 0 to 99 loop
      a <= std_logic_vector(to_unsigned(i * 7 mod 256, 8));
      b <= std_logic_vector(to_unsigned(i * 13 mod 256, 8));
      if i mod 2 = 1 then
        cin <= '1';
      else
        cin <= '0';
      end if;
      wait for 1 ns;
      acc := acc + to_integer(ss) + to_integer(unsigned(hi));
    end loop;
    report "acc=" & integer'image(acc);
    wait;
  end process;
end architecture;
