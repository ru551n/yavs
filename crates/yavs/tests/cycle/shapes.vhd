-- Clocked and combinational shapes of cycle mode against the normal mode: a `wait until` process
-- with an enable, an asynchronous reset, variables with initial values, an array of vectors read
-- and written at run-time indices, concurrent simple, conditional and selected assignments, a
-- `case` on a vector, an unrolled loop, slices chosen at run time, constants and generics.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity cycle_shapes is
  generic (N : positive := 4; CYCLES : positive := 40);
end entity;
architecture a of cycle_shapes is
  constant K : natural := 3;
  constant MAGIC : unsigned(7 downto 0) := x"A5";
  signal clk, rst : std_ulogic := '0';
  signal din : unsigned(7 downto 0) := (others => '0');
  type arr_t is array (0 to N - 1) of unsigned(7 downto 0);
  signal regs : arr_t := (others => (others => '0'));
  signal acc : unsigned(7 downto 0) := (others => '0');
  signal cnt : integer range 0 to 255 := 0;
  signal en : std_ulogic := '0';
  signal sel : natural range 0 to N - 1 := 0;
  signal pick, mux, sum, dec : unsigned(7 downto 0);
  signal flag : boolean;
  signal code : std_ulogic_vector(1 downto 0);
  signal q : std_ulogic := '0';
  signal wide : std_ulogic_vector(8 * N - 1 downto 0) := (others => '0');
  signal byte_out : std_ulogic_vector(7 downto 0);
begin
  w : process
  begin
    wait until rising_edge(clk) and en = '1';
    regs(sel) <= din;
  end process;
  ar : process (clk, rst)
    variable v : unsigned(7 downto 0) := to_unsigned(K, 8);
  begin
    if rst = '1' then
      acc <= (others => '0');
      cnt <= 0;
      v := to_unsigned(K, 8);
    elsif rising_edge(clk) then
      v := v + 1;
      acc <= acc + regs(sel) + v;
      if cnt = 255 then
        cnt <= 0;
      else
        cnt <= cnt + 1;
      end if;
    end if;
  end process;
  q <= din(0) when rising_edge(clk);
  ws : process (clk)
  begin
    if rising_edge(clk) then
      wide(8 * sel + 7 downto 8 * sel) <= std_ulogic_vector(din);
    end if;
  end process;
  byte_out <= wide(8 * sel + 7 downto 8 * sel) xor wide(8 * N - 1 downto 8 * N - 8);
  mux <= regs(0) when sel = 0 else regs(1) xor regs(N - 1) when sel = 1 else not acc;
  pick <= regs(sel);
  flag <= acc > 100;
  with acc(1 downto 0) select code <= "11" when "00", "10" when "01", "01" when others;
  sums : process (regs)
    variable s : unsigned(7 downto 0);
  begin
    s := MAGIC;
    for i in regs'range loop
      s := s xor regs(i);
    end loop;
    sum <= s;
  end process;
  decode : process (all)
  begin
    case std_ulogic_vector(acc(7 downto 6)) is
      when "00" => dec <= acc;
      when "01" => dec <= acc + 1;
      when "10" => dec <= shift_right(acc, 1);
      when others => dec <= (others => '1');
    end case;
  end process;
  main : process
    variable x : unsigned(15 downto 0) := x"ACE1";
  begin
    for c in 1 to CYCLES loop
      x := x(14 downto 0) & (x(15) xor x(13) xor x(12) xor x(10));
      din <= x(7 downto 0);
      en <= x(9);
      sel <= to_integer(x(12 downto 11)) mod N;
      rst <= '1' when c = 3 or c = 17 else '0';
      wait for 2 ns;
      clk <= '1';
      wait for 5 ns;
      clk <= '0';
      wait for 3 ns;
      report "c=" & integer'image(c) & " acc=" & to_hstring(acc) & " cnt=" & integer'image(cnt)
        & " mux=" & to_hstring(mux) & " pick=" & to_hstring(pick) & " flag=" & boolean'image(flag)
        & " code=" & to_string(code) & " sum=" & to_hstring(sum) & " dec=" & to_hstring(dec)
        & " q=" & std_ulogic'image(q) & " r0=" & to_hstring(regs(0)) & " r3=" & to_hstring(regs(N - 1))
        & " wide=" & to_hstring(wide) & " bo=" & to_hstring(byte_out);
    end loop;
    wait;
  end process;
end architecture;
