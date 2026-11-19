-- Narrow lanes of packed vectors (`W` replaced by a width): a delay chain of lanes, lanes copied in
-- place and from two sources in turn, in-place shifts of a variable in both directions and a swap,
-- lanes of 8 and 5 bits (fields cross words) and of `W` bits, sign-extended lanes multiplied.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity cycle_lanes is
end entity;
architecture t of cycle_lanes is
  type b8_t is array (0 to 9) of signed(7 downto 0);
  type b5_t is array (0 to 14) of unsigned(4 downto 0);
  type bw_t is array (0 to 4) of std_ulogic_vector(W - 1 downto 0);
  type p16_t is array (0 to 9) of signed(15 downto 0);
  signal clk : std_ulogic := '0';
  signal din : std_ulogic_vector(W + 12 downto 0) := (others => '0');
  signal c8, d8, e8 : b8_t := (others => (others => '0'));
  signal c5, d5 : b5_t := (others => (others => '0'));
  signal cw, dw, ew : bw_t := (others => (others => '0'));
  signal p : p16_t := (others => (others => '0'));
  signal sum : unsigned(15 downto 0) := (others => '0');
begin
  regs : process (clk)
    variable v8 : b8_t := (others => (others => '0'));
    variable u5 : b5_t := (others => (others => '0'));
    variable vw : bw_t := (others => (others => '0'));
    variable t8 : signed(7 downto 0);
    variable s : unsigned(15 downto 0);
  begin
    if rising_edge(clk) then
      -- A delay chain: lane k from lane k - 1.
      c8(0) <= signed(din(7 downto 0));
      c5(0) <= unsigned(din(12 downto 8));
      cw(0) <= din(W + 12 downto 13);
      for k in 1 to 9 loop
        c8(k) <= c8(k - 1);
      end loop;
      for k in 1 to 14 loop
        c5(k) <= c5(k - 1);
      end loop;
      for k in 1 to 4 loop
        cw(k) <= cw(k - 1);
      end loop;
      -- In place, and from two sources in turn (even lanes, odd lanes), and one computed lane
      -- between copied ones.
      for k in 0 to 9 loop
        d8(k) <= c8(k);
        if k mod 2 = 0 then
          e8(k) <= c8(k);
        else
          e8(k) <= d8(k);
        end if;
      end loop;
      e8(4) <= c8(4) xor d8(4);
      for k in 0 to 14 loop
        d5(k) <= c5(14 - k);
      end loop;
      for k in 0 to 4 loop
        dw(k) <= cw(k);
        ew(k) <= cw((k + 1) mod 5);
      end loop;
      -- In-place shifts of a variable: up from the top (each lane its lower neighbour's old value)
      -- and from the bottom (lane 0 spreads), and a swap.
      v8 := c8;
      for k in 9 downto 1 loop
        v8(k) := v8(k - 1);
      end loop;
      u5 := c5;
      for k in 1 to 14 loop
        u5(k) := u5(k - 1);
      end loop;
      vw := cw;
      for k in 0 to 3 loop
        vw(k) := vw(k + 1);
      end loop;
      t8 := v8(2);
      v8(2) := v8(3);
      v8(3) := t8;
      v8(5) := v8(6);
      v8(6) := v8(5);
      for k in 0 to 9 loop
        p(k) <= v8(k) * c8(9 - k);
      end loop;
      s := (others => '0');
      for k in 0 to 14 loop
        s := s + u5(k);
      end loop;
      for k in 0 to 4 loop
        s := s + unsigned(vw(k)(W - 1 downto W - 1)) + unsigned(vw(k)(0 downto 0));
      end loop;
      sum <= s;
    end if;
  end process;
  main : process
    variable r : unsigned(W + 12 downto 0) := (others => '1');
    variable h : unsigned(31 downto 0);
  begin
    for k in 1 to 16 loop
      r := r(W + 11 downto 0) & (r(W + 12) xor r((W + 12) / 2) xor r(1) xor r(0));
      din <= std_ulogic_vector(r);
      wait for 1 ns;
      clk <= '1';
      wait for 5 ns;
      clk <= '0';
      wait for 4 ns;
      h := (others => '0');
      for j in 0 to 9 loop
        h := (h(26 downto 0) & h(31 downto 27)) xor resize(unsigned(c8(j)), 32)
          xor shift_left(resize(unsigned(d8(j)), 32), 8) xor shift_left(resize(unsigned(e8(j)), 32), 16)
          xor resize(unsigned(p(j)), 32);
      end loop;
      for j in 0 to 14 loop
        h := (h(26 downto 0) & h(31 downto 27)) xor resize(c5(j), 32) xor shift_left(resize(d5(j), 32), 5);
      end loop;
      for j in 0 to 4 loop
        h := (h(26 downto 0) & h(31 downto 27)) xor resize(unsigned(cw(j)), 32)
          xor resize(unsigned(dw(j)) srl 1, 32) xor resize(unsigned(ew(j)) srl 2, 32);
      end loop;
      report "k=" & integer'image(k) & " h=" & to_hstring(h) & " sum=" & to_hstring(sum)
        & " e4=" & to_hstring(e8(4)) & " c5=" & to_hstring(c5(14)) & " v=" & to_hstring(p(3));
    end loop;
    wait;
  end process;
end architecture;
