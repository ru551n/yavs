-- Concurrent copies of a 200-element bus (blocks of 64 and a part), of slices, of an integer and of
-- a record field, the inputs changed 30 times: the kernel commits each copy itself (#519), with no
-- batch run per copy; a reader of every stage checks the values arrive one delta apart.
library ieee; use ieee.std_logic_1164.all; use ieee.numeric_std.all;
entity copy_forward is end;
architecture a of copy_forward is
  type rec is record
    v : std_logic_vector(7 downto 0);
    n : integer;
  end record;
  signal b0, b1, b2 : std_logic_vector(199 downto 0) := (others => '0');
  signal lo : std_logic_vector(69 downto 0) := (others => '0');
  signal i0, i1 : integer := 0;
  signal r0, r1 : rec := ((others => '0'), 0);
  signal f : std_logic_vector(7 downto 0) := (others => '0');
begin
  b1 <= b0;
  b2 <= b1;
  lo <= b2(129 downto 60);
  i1 <= i0;
  r1 <= r0;
  f <= r1.v;
  process
    variable s : natural := 0;
  begin
    for k in 1 to 30 loop
      b0 <= std_logic_vector(rotate_left(unsigned(b0), 7)) xor std_logic_vector(to_unsigned(k * 37, 200));
      i0 <= k * 3;
      r0 <= (std_logic_vector(to_unsigned(k * 11 mod 256, 8)), k);
      wait for 0 ns;
      assert b1 /= b0 severity failure;
      wait for 0 ns;
      assert b1 = b0 and b2 /= b1 and i1 = i0 and r1 = r0 severity failure;
      wait for 1 ns;
      s := (s + to_integer(unsigned(lo(30 downto 0))) mod 1000003 + i1 + r1.n + to_integer(unsigned(f))) mod 1000003;
    end loop;
    report "s=" & integer'image(s);
    wait;
  end process;
end;
