-- difftest: top=record_var_paths
-- Record variables of processes on every path shape the optimiser's dead-byte pass meets
-- (`v := r` first, reads before it, state kept across activations, partial writes, procedures
-- taking the variable, waits in a procedure, loops, case arms, wide vectors, arrays of records).
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

entity record_var_paths is
end entity;

architecture a of record_var_paths is
  type rec is record
    a   : std_ulogic;
    b   : std_ulogic_vector(7 downto 0);
    w   : std_ulogic_vector(69 downto 0);
    n   : integer range 0 to 1000;
    sub : std_ulogic_vector(3 downto 0);
  end record;
  constant REC0 : rec := ('0', x"00", (others => '0'), 0, "0000");
  type rec_arr is array (0 to 3) of rec;

  signal clk  : std_ulogic := '0';
  signal cyc  : natural := 0;
  signal r, rin, s, sin, q, qin : rec := REC0;
  signal ra, rain : rec_arr := (others => REC0);
  signal done : boolean := false;

  function mix(x : rec) return natural is
  begin
    return (to_integer(unsigned(x.b)) * 7 + to_integer(unsigned(x.w(69 downto 60))) * 3
            + to_integer(unsigned(x.w(9 downto 0))) + x.n * 11 + to_integer(unsigned(x.sub))
            + std_ulogic'pos(x.a)) mod 100000;
  end function;

  procedure bump(variable x : inout rec) is
  begin
    x.n := (x.n + to_integer(unsigned(x.b))) mod 1000;
    x.sub := x.sub(2 downto 0) & x.a;
  end procedure;
begin
  clk <= not clk after 5 ns when not done;

  regs : process (clk)
  begin
    if rising_edge(clk) then
      r <= rin;
      s <= sin;
      q <= qin;
      ra <= rain;
      cyc <= cyc + 1;
    end if;
  end process;

  -- The idiom with defaults overwritten, a case, a wide vector written in two pieces or one.
  comb_r : process (r, cyc)
    variable v : rec;
  begin
    v := r;
    v.a := '0';
    v.b := x"22";
    v.n := r.n + 2;
    v.b := x"11";
    if cyc mod 3 = 0 then
      v.a := '1';
    end if;
    case cyc mod 4 is
      when 0 => v.b := std_ulogic_vector(unsigned(r.b) + 3);
      when 1 => v.w(69 downto 64) := std_ulogic_vector(to_unsigned(cyc mod 64, 6));
                v.w(63 downto 0) := (others => r.a);
      when 2 => v.w(69 downto 64) := "101010";
      when others => v.n := (r.n + 1) mod 1000;
    end case;
    v.sub := REC0.sub;
    if cyc mod 5 = 1 then
      v.sub := std_ulogic_vector(to_unsigned(cyc mod 16, 4));
    end if;
    bump(v);
    rin <= v;
  end process;

  -- A variable read before `v := s` on one path, assigned only on some paths (it keeps its
  -- value), and partly written before the whole assignment.
  comb_s : process (s, cyc)
    variable v    : rec;
    variable seen : natural := 0;
  begin
    if cyc mod 2 = 0 then
      seen := (seen + mix(v)) mod 100000;
    end if;
    if cyc mod 3 = 1 then
      v.b := x"5a";
    end if;
    if cyc mod 7 /= 3 then
      v := s;
    end if;
    v.n := (v.n + seen) mod 1000;
    v.w(5 downto 0) := v.b(5 downto 0);
    sin <= v;
  end process;

  -- A procedure that waits and reads the variable it was given, and `v := q` in a loop.
  waiter : process
    variable v   : rec;
    variable acc : natural := 0;
    procedure step(variable x : inout rec) is
    begin
      x.a := not x.a;
      wait until rising_edge(clk);
      x.b := std_ulogic_vector(unsigned(x.b) + resize(unsigned(x.sub), 8));
    end procedure;
  begin
    v := q;
    v.n := 0;
    step(v);
    for i in 0 to 2 loop
      v := q;
      v.n := (i * 5 + cyc) mod 1000;
      acc := (acc + mix(v)) mod 100000;
    end loop;
    v.sub := std_ulogic_vector(to_unsigned(acc mod 16, 4));
    qin <= v;
    wait until rising_edge(clk);
  end process;

  -- An array of records indexed by a variable.
  comb_a : process (ra, cyc)
    variable va : rec_arr;
    variable k  : natural;
  begin
    va := ra;
    k := cyc mod 4;
    va(k).n := (va(k).n + k + 1) mod 1000;
    va(k).b := std_ulogic_vector(unsigned(va((k + 1) mod 4).b) + 1);
    va((k + 2) mod 4) := REC0;
    va((k + 2) mod 4).a := '1';
    rain <= va;
  end process;

  check : process (clk)
    variable total : natural := 0;
  begin
    if falling_edge(clk) then
      total := (total + mix(r) + 3 * mix(s) + 5 * mix(q) + 7 * mix(ra(0)) + 11 * mix(ra(1))
                + 13 * mix(ra(2)) + 17 * mix(ra(3))) mod 1000000;
      report "cycle " & integer'image(cyc) & " r " & integer'image(mix(r)) & " s " & integer'image(mix(s))
        & " q " & integer'image(mix(q)) & " a " & integer'image(mix(ra(cyc mod 4))) & " total " & integer'image(total);
      if cyc = 40 then
        done <= true;
      end if;
    end if;
  end process;
end architecture;
