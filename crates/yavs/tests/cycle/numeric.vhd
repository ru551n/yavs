-- The numeric_std operations of cycle mode against the normal mode on 0/1 inputs, for a width that
-- the test fills in (with the marked lines where the operations that take one word only go). Every
-- combinational process assigns its outputs on every path.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity cycle_ops2 is
end entity;
architecture a of cycle_ops2 is
  signal clk : std_ulogic := '0';
  signal a, b : std_ulogic_vector(W - 1 downto 0) := (others => '0');
  signal k : natural range 0 to 2 * W := 0;
  signal n : integer range -9 to 9 := 0;
  signal u_add, u_sub, s_add, s_sub, u_addi, s_subi, u_mx, s_mn : unsigned(W - 1 downto 0);
  signal u_mul, s_mul : unsigned(2 * W - 1 downto 0);
  signal u_ext, s_ext : unsigned(W + 3 downto 0);
  signal u_cut, s_cut : unsigned(W / 2 downto 0);
  signal sl, sr, sra1, rl, rr, slk, srk, srak, rlk, rrk : unsigned(W - 1 downto 0);
  signal neg, absv : unsigned(W - 1 downto 0);
  signal cmp : std_ulogic_vector(15 downto 0);
  signal red : std_ulogic_vector(5 downto 0);
  signal fsm_out : std_ulogic_vector(1 downto 0);
  signal q_div, q_mod, q_rem, s_div, s_mod, s_rem : unsigned(W - 1 downto 0);
  signal ti : integer range -2 ** 20 to 2 ** 20 := 0;
  signal tu : unsigned(W - 1 downto 0);
  type state_t is (idle, run, hold, done_s);
  signal st : state_t := idle;
  signal r : unsigned(W - 1 downto 0) := (others => '0');
begin
  arith : process (all)
  begin
    u_add <= unsigned(a) + unsigned(b(W - 1 downto W / 2));
    u_sub <= unsigned(a) - unsigned(b);
    s_add <= unsigned(signed(a) + signed(b(W / 2 downto 0)));
    s_sub <= unsigned(signed(a) - signed(b));
    u_addi <= unsigned(a) + k;
    s_subi <= unsigned(signed(a) - n);
    u_mul <= unsigned(a) * unsigned(b);
    s_mul <= unsigned(signed(a) * signed(b));
    u_ext <= resize(unsigned(a), W + 4);
    s_ext <= unsigned(resize(signed(a), W + 4));
    u_cut <= resize(unsigned(a), W / 2 + 1);
    s_cut <= unsigned(resize(signed(a), W / 2 + 1));
    u_mx <= maximum(unsigned(a), unsigned(b));
    s_mn <= unsigned(minimum(signed(a), signed(b)));
    neg <= unsigned(-signed(a));
    absv <= unsigned(abs signed(a));
  end process;
  cmps : process (all)
  begin
    cmp <= (others => '0');
    if unsigned(a) < unsigned(b) then cmp(0) <= '1'; end if;
    if unsigned(a) <= unsigned(b(W - 1 downto W / 2)) then cmp(1) <= '1'; end if;
    if signed(a) > signed(b) then cmp(2) <= '1'; end if;
    if signed(a) >= signed(b) then cmp(3) <= '1'; end if;
    if unsigned(a) = unsigned(b) then cmp(4) <= '1'; end if;
    if signed(a) /= signed(b(W / 2 downto 0)) then cmp(5) <= '1'; end if;
    if unsigned(a) > k then cmp(6) <= '1'; end if;
    if signed(a) < n then cmp(7) <= '1'; end if;
    if k <= unsigned(b) then cmp(8) <= '1'; end if;
    if a = b then cmp(9) <= '1'; end if;
    if std_match(a(0 downto 0), "-") then cmp(10) <= '1'; end if;
    if (a(0) = '1' and b(0) = '0') or a(W - 1) = '1' then cmp(11) <= '1'; end if;
    cmp(12) <= a(0) ?= b(0);
  end process;
  shifts : process (all)
  begin
    sl <= shift_left(unsigned(a), 3);
    sr <= shift_right(unsigned(a), W - 1);
    sra1 <= unsigned(shift_right(signed(a), 2));
    rl <= rotate_left(unsigned(a), W + 1);
    rr <= rotate_right(unsigned(a), 5);
--DYN
  end process;
  reds : process (all)
  begin
    red <= (and a) & (or a) & (xor a) & (nand b) & (nor b) & (xnor b);
  end process;
--DIV
  fsm : process (clk)
  begin
    if rising_edge(clk) then
      case st is
        when idle => if a(0) = '1' then st <= run; end if;
        when run => st <= hold; r <= r + unsigned(b);
        when hold => if b(0) = '1' then st <= done_s; else st <= run; end if;
        when done_s => st <= idle; r <= (others => '0');
      end case;
    end if;
  end process;
  with st select fsm_out <= "00" when idle, "01" when run, "10" when hold, "11" when others;
  main : process
    variable x : unsigned(63 downto 0) := x"9E3779B97F4A7C15";
    variable va, vb : std_ulogic_vector(W - 1 downto 0);
  begin
    for c in 1 to 16 loop
      for j in 0 to W - 1 loop
        va(j) := x(j mod 64) xor x((j * 7 + 3) mod 64);
        vb(j) := x((j * 5 + 1) mod 64);
      end loop;
      if c mod 4 = 0 then vb := va; end if;
      if c mod 5 = 0 then vb := (others => '0'); end if;
      if c mod 7 = 0 then va := (others => '1'); end if;
      a <= va;
      b <= vb;
      k <= c mod (2 * W + 1);
      n <= (c mod 19) - 9;
      x := (x(62 downto 0) & '0') xor (x(63 downto 0) and x"000000000000001B" and (63 downto 0 => x(63)));
      x := x + 16#5851F42D#;
      clk <= '1';
      wait for 5 ns;
      clk <= '0';
      wait for 5 ns;
      report "c=" & integer'image(c) & " ua=" & to_hstring(u_add) & " us=" & to_hstring(u_sub)
        & " sa=" & to_hstring(s_add) & " ss=" & to_hstring(s_sub) & " ui=" & to_hstring(u_addi)
        & " si=" & to_hstring(s_subi) & " um=" & to_hstring(u_mul) & " sm=" & to_hstring(s_mul)
        & " ue=" & to_hstring(u_ext) & " se=" & to_hstring(s_ext) & " uc=" & to_hstring(u_cut)
        & " sc=" & to_hstring(s_cut) & " mx=" & to_hstring(u_mx) & " mn=" & to_hstring(s_mn)
        & " neg=" & to_hstring(neg) & " abs=" & to_hstring(absv) & " cmp=" & to_string(cmp)
        & " sl=" & to_hstring(sl) & " sr=" & to_hstring(sr) & " sra=" & to_hstring(sra1)
        & " rl=" & to_hstring(rl) & " rr=" & to_hstring(rr) & " red=" & to_string(red)
        & " st=" & state_t'image(st) & " fo=" & to_string(fsm_out) & " r=" & to_hstring(r)
        --PRINT
        ;
    end loop;
    wait;
  end process;
end architecture;
