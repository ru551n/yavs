-- Bits, slices and elements of constant arrays of vectors in cycle mode against the normal mode, for a
-- width the test fills in (and the string literals PAT1 to PAT3 of that width): an element has the index
-- range of its subtype, not the one of the literal or function result that made it (`0 to n - 1`).
library ieee;
use ieee.std_logic_1164.all;
package cycle_constelem_pkg is
  subtype ed_t is std_ulogic_vector(W + 2 downto 3);
  subtype eu_t is std_ulogic_vector(3 to W + 2);
  subtype e0_t is std_ulogic_vector(W - 1 downto 0);
  type ad_t is array (natural range <>) of ed_t;
  type au_t is array (natural range <>) of eu_t;
  type a0_t is array (natural range <>) of e0_t;
  type ao_t is array (natural range <>) of std_ulogic_vector;
  type rec_t is record
    n : integer;
    a : ed_t;
  end record;
  type ra_t is array (natural range <>) of rec_t;
  type m_t is array (0 to 1) of au_t(2 to 4);
  type box_t is record
    n : integer;
    f : ad_t(4 downto 2);
  end record;
  type b8_t is array (natural range <>) of std_ulogic_vector(7 downto 0);
  -- A pattern of `n` elements in a range `0 to n - 1`, bit 0 set.
  function pat (n, seed : natural) return std_ulogic_vector;
  constant pk_c : ad_t(4 downto 2);
end package;

package body cycle_constelem_pkg is
  function pat (n, seed : natural) return std_ulogic_vector is
    variable v : std_ulogic_vector(0 to n - 1) := (others => '0');
  begin
    for i in v'range loop
      if (i * i * seed + i + seed) mod 7 < 3 then
        v(i) := '1';
      end if;
    end loop;
    v(0) := '1';
    return v;
  end function;
  constant pk_c : ad_t(4 downto 2) := (pat(W, 1), pat(W, 2), pat(W, 3));
end package body;

library ieee;
use ieee.std_logic_1164.all;
use work.cycle_constelem_pkg.all;
entity cycle_constelem is
  generic (
    g : au_t(2 to 4) := (pat(W, 4), pat(W, 5), pat(W, 6))
  );
end entity;
architecture a of cycle_constelem is
  constant cd_c : ad_t(2 to 4) := (pat(W, 7), pat(W, 8), pat(W, 9));
  constant cu_c : au_t(4 downto 2) := (pat(W, 10), pat(W, 11), pat(W, 12));
  constant c0_c : a0_t(2 to 4) := (pat(W, 13), pat(W, 14), pat(W, 15));
  constant co_c : ao_t(2 to 4)(W + 2 downto 3) := (pat(W, 16), pat(W, 17), pat(W, 18));
  constant rc_c : ra_t(2 to 4) := ((5, pat(W, 19)), (-3, pat(W, 20)), (9, pat(W, 21)));
  constant m_c : m_t := ((pat(W, 22), pat(W, 23), pat(W, 24)), (pat(W, 25), pat(W, 26), pat(W, 27)));
  constant box_c : box_t := (n => 1, f => (pat(W, 28), pat(W, 29), pat(W, 30)));
  constant x8_c : b8_t(0 to 1) := (x"9a", x"01");
  constant lit_c : ad_t(2 to 4) := (PAT2, PAT3, PAT1);
  function elem_bit (t : ad_t; i, b : integer) return std_ulogic is
  begin
    return t(i)(b);
  end function;
  function sel (e : std_ulogic) return ad_t is
  begin
    if e = '1' then
      return (PAT1, PAT2, PAT3);
    end if;
    return (PAT3, PAT2, PAT1);
  end function;
  signal clk : std_ulogic := '0';
  signal ix : integer range 2 to 4 := 2;
  signal jx : integer range 0 to 1 := 0;
  signal bx : integer range 3 to W + 2 := 3;
  signal sd, sp, so, sb, ss, sw, st : ad_t(2 to 4) := (others => (others => '0'));
  signal su, sg, sm1, sm0, sl : au_t(2 to 4) := (others => (others => '0'));
  signal s0 : a0_t(2 to 4) := (others => (others => '0'));
  signal sr : ra_t(2 to 4) := (others => (0, (others => '0')));
  signal s8 : b8_t(0 to 1) := (others => x"00");
  signal ed, sa, sf : ed_t := (others => '0');
  signal hi, lo, a_r, a2, cz : std_ulogic := '0';
begin
  clk <= not clk after 5 ns when now < (6 * W + 12) * 10 ns else clk;

  count : process (clk)
  begin
    if rising_edge(clk) then
      a_r <= not a_r;
      if bx = W + 2 then
        bx <= 3;
        if ix = 4 then
          ix <= 2;
          jx <= 1 - jx;
        else
          ix <= ix + 1;
        end if;
      else
        bx <= bx + 1;
      end if;
    end if;
  end process;

  -- Logic whose dependence the analysis takes from a bit of a constant, listed before the logic it
  -- reads: with the bit read from the wrong end it would run first, on the old value.
  pick : process (all)
  begin
    if x8_c(1)(0) = '1' then
      cz <= a2;
    else
      cz <= '0';
    end if;
  end process;

  inv : process (all)
  begin
    a2 <= not a_r;
  end process;

  -- Bits at static indices, the original register-file default style at loop indices.
  stat : process (clk)
  begin
    if rising_edge(clk) then
      hi <= cd_c(3)(W + 2);
      lo <= cu_c(3)(3);
    end if;
  end process;

  -- Element ranges from 0 (the reproducer): read reversed when a literal's range was taken.
  zero : process (clk)
  begin
    if rising_edge(clk) then
      for b in 0 to 1 loop
        for i in 0 to 7 loop
          s8(b)(i) <= x8_c(b)(i);
        end loop;
      end loop;
      for k in 2 to 4 loop
        for b in 0 to W - 1 loop
          s0(k)(b) <= c0_c(k)(b);
        end loop;
      end loop;
    end if;
  end process;

  loops : process (clk)
  begin
    if rising_edge(clk) then
      for k in cd_c'range loop
        for b in cd_c(k)'range loop
          sd(k)(b) <= cd_c(k)(b);
          sp(k)(b) <= pk_c(k)(b);
        end loop;
        for b in co_c(k)'range loop
          so(k)(b) <= co_c(k)(b);
        end loop;
        for b in 3 to W + 2 loop
          su(k)(b) <= cu_c(k)(b);
          sg(k)(b) <= g(k)(b);
          sb(k)(b) <= box_c.f(k)(b);
        end loop;
        for b in lit_c(k)'range loop
          st(k)(b) <= lit_c(k)(b);
        end loop;
      end loop;
    end if;
  end process;

  -- Indices chosen at run time: a bit of each, an element, a field, two levels.
  runtime : process (clk)
  begin
    if rising_edge(clk) then
      ss(ix)(bx) <= cd_c(ix)(bx);
      sw(ix) <= co_c(ix);
      sr(ix).a(bx) <= rc_c(ix).a(bx);
      sr(ix).n <= rc_c(ix).n;
      if jx = 0 then
        sm0(ix)(bx) <= m_c(jx)(ix)(bx);
      else
        sm1(ix)(bx) <= m_c(jx)(ix)(bx);
      end if;
    end if;
  end process;

  -- Aggregates of string literals: an actual of an unconstrained formal, a function's result.
  aggs : process (clk)
  begin
    if rising_edge(clk) then
      for b in 3 to W + 2 loop
        sa(b) <= elem_bit((PAT1, PAT2, PAT3), 1, b);
        sf(b) <= sel(a_r)(0)(b);
      end loop;
    end if;
  end process;

  -- A process constant and a constant of a procedure it calls; slices.
  local : process (clk)
    constant pc_c : au_t(2 to 4) := (PAT1, PAT2, PAT3);
    procedure copy (signal t : out ed_t) is
      constant fc_c : ad_t(2 to 4) := (PAT3, PAT1, PAT2);
    begin
      for b in fc_c(3)'range loop
        t(b) <= fc_c(3)(b);
      end loop;
    end procedure;
  begin
    if rising_edge(clk) then
      for k in 2 to 4 loop
        sl(k)(3 to W + 2) <= pc_c(k)(3 to W + 2);
        sl(k)(3) <= pc_c(k)(3);
      end loop;
      copy(ed);
    end if;
  end process;

  main : process
  begin
    wait for (6 * W + 10) * 10 ns;
    for k in 2 to 4 loop
      report "k=" & integer'image(k) & " sd=" & to_hstring(sd(k)) & " sp=" & to_hstring(sp(k))
        & " so=" & to_hstring(so(k)) & " su=" & to_hstring(su(k)) & " sg=" & to_hstring(sg(k))
        & " sb=" & to_hstring(sb(k)) & " s0=" & to_hstring(s0(k)) & " st=" & to_hstring(st(k));
      report "k=" & integer'image(k) & " ss=" & to_hstring(ss(k)) & " sw=" & to_hstring(sw(k))
        & " sr=" & integer'image(sr(k).n) & " " & to_hstring(sr(k).a) & " sm0=" & to_hstring(sm0(k))
        & " sm1=" & to_hstring(sm1(k)) & " sl=" & to_hstring(sl(k));
    end loop;
    report "hi=" & std_ulogic'image(hi) & " lo=" & std_ulogic'image(lo) & " s8=" & to_hstring(s8(0))
      & to_hstring(s8(1)) & " ed=" & to_hstring(ed) & " cz=" & std_ulogic'image(cz) & " a_r="
      & std_ulogic'image(a_r) & " sa=" & to_hstring(sa) & " sf=" & to_hstring(sf);
    wait;
  end process;
end architecture;
