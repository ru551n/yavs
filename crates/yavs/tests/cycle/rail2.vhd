-- The second image of the metavalue contract (no --two-state) (A38, A43) beside the normal mode: a memory of words
-- without start values (one never written, one written from an input that becomes 'U'), a call
-- whose local starts 'U', combinational choices on a register still 'U' (`when ... else`, also
-- on `??`, and `to_integer` as an index, which the normal mode reads as 0), clocked choices on
-- it (`if`, `case`, a `numeric_std` compare: 'U' there, clocked members read their own image),
-- a register taking an input that becomes 'U' while its own value stays, and 'X' start values
-- alone and mixed with 'U' ones. `xx` chooses on a variable copied from a 'U' (the second image
-- takes the other branch: 'U' where the normal mode has `b`); `zz` compares `xx` with `b` as the
-- region's image holds it, so it shows the normal mode's `a`, not a 'U'. `z` chooses on `t` at
-- the edge where it first gets a value. `k` (a choice on a variable copied from a 'U') and `k2`
-- (`??` of a 'U') keep their start values as the normal mode. With LATE the 'X' start value is
-- a 'U' and an 'X' reaches `xin` later.
library ieee; use ieee.std_logic_1164.all; use ieee.numeric_std.all;
entity cycle_rail2 is
  generic (W : positive := 8; LATE : boolean := false);
end;
architecture a of cycle_rail2 is
  signal clk : std_ulogic := '0';
  type mem_t is array (0 to 1023) of std_ulogic_vector(W - 1 downto 0);
  signal mem : mem_t;
  signal wa : unsigned(9 downto 0) := (others => '0');
  signal d : std_ulogic_vector(W - 1 downto 0) := (others => '1');
  signal one : std_ulogic := '1';
  signal f : std_ulogic_vector(W - 1 downto 0);
  function low(x : std_ulogic) return std_ulogic_vector is
    variable v : std_ulogic_vector(W - 1 downto 0);
  begin
    v(0) := x;
    return v;
  end function;
  signal s : std_ulogic;
  signal ad : std_ulogic_vector(1 downto 0);
  signal a : std_ulogic_vector(W - 1 downto 0) := (others => '1');
  signal b : std_ulogic_vector(W - 1 downto 0) := (others => '0');
  signal y, y2, w2, c, r, lt, r0, lt0 : std_ulogic_vector(W - 1 downto 0);
  type small_t is array (0 to 2) of std_ulogic_vector(W - 1 downto 0);
  signal small : small_t := ((others => '1'), (others => '0'), (others => '1'));
  signal rd : std_ulogic_vector(W - 1 downto 0);
  signal din : std_ulogic_vector(W - 1 downto 0) := (others => '0');
  signal q : std_ulogic_vector(W - 1 downto 0);
  function first(late : boolean) return std_ulogic is
  begin
    if late then
      return 'U';
    end if;
    return 'X';
  end function;
  signal x : std_ulogic_vector(W - 1 downto 0) := (others => first(LATE));
  signal xin : std_ulogic_vector(W - 1 downto 0) := (others => '0');
  signal k, k2 : std_ulogic_vector(W - 1 downto 0) := (others => '0');
  signal qxi : std_ulogic_vector(W - 1 downto 0);
  signal u : std_ulogic_vector(W - 1 downto 0);
  signal qx, qa, qo, z : std_ulogic_vector(W - 1 downto 0);
  signal en : std_ulogic := '0';
  signal t : std_ulogic;
  signal xx, zz : std_ulogic_vector(W - 1 downto 0);
begin
  clk <= not clk after 5 ns when now < 1 us else clk;
  y <= a when s = '1' else b;
  w2 <= a when s /= '1' else b;
  y2 <= a when s else b;
  rd <= small(to_integer(unsigned(ad)));
  cv : process (s, a, b)
    variable v : std_ulogic;
  begin
    v := s;
    if v = '1' then
      xx <= a;
    else
      xx <= b;
    end if;
  end process;
  zz <= a when xx = b else b;
  wr : process (clk) begin
    if rising_edge(clk) then
      mem(to_integer(wa)) <= d;
      f <= low(one);
    end if;
  end process;
  ch : process (clk) begin
    if rising_edge(clk) then
      if s = '1' then
        c <= a;
      else
        c <= b;
      end if;
      case ad is
        when "11" | "10" => r <= a;
        when others => r <= b;
      end case;
      if unsigned(ad) > 1 then
        lt <= a;
      else
        lt <= b;
      end if;
      case ad is
        when "00" | "01" => r0 <= a;
        when others => r0 <= b;
      end case;
      if unsigned(ad) < 2 then
        lt0 <= a;
      else
        lt0 <= b;
      end if;
      q <= din;
      qx <= x;
      qa <= x and u;
      qo <= x or u;
      if en = '1' then
        t <= '1';
      end if;
      if t = '1' then
        z <= a;
      else
        z <= b;
      end if;
    end if;
  end process;
  kp : process (clk)
    variable v : std_ulogic;
  begin
    if rising_edge(clk) then
      v := s;
      if v = '1' then
        k <= a;
      end if;
      if s then
        k2 <= a;
      end if;
    end if;
  end process;
  xp : process (clk) begin
    if rising_edge(clk) then
      qxi <= xin;
    end if;
  end process;
  stim : process
    procedure show (tag : string) is
    begin
      report tag & " m0=" & to_string(mem(0)) & " m1=" & to_string(mem(1)) & " f=" & to_string(f)
        & " y=" & to_string(y) & " y2=" & to_string(y2) & " w2=" & to_string(w2) & " c=" & to_string(c) & " r=" & to_string(r)
        & " lt=" & to_string(lt) & " r0=" & to_string(r0) & " lt0=" & to_string(lt0) & " rd=" & to_string(rd) & " q=" & to_string(q)
        & " qx=" & to_string(qx) & " qa=" & to_string(qa) & " qo=" & to_string(qo)
        & " z=" & to_string(z) & " k=" & to_string(k) & " k2=" & to_string(k2)
        & " qxi=" & to_string(qxi) & " xx=" & to_string(xx) & " zz=" & to_string(zz);
    end procedure;
  begin
    wait for 22 ns; show("a");
    wa <= to_unsigned(1, 10); d(0) <= 'U'; din(0) <= 'U'; en <= '1';
    if LATE then
      xin <= (others => 'X');
    end if;
    wait for 5 ns; show("e");
    wait for 15 ns; show("b");
    wait;
  end process;
end;
