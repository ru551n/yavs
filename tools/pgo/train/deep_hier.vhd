-- pgo: top=deep_hier_tb std=2008 stop=680 ns analyze=3
-- Reproduces an elaboration-heavy design: seven levels, for-generate and if-generate at several
-- levels, about 3000 instances, computed generics, sliced and converted port maps, open ports,
-- component default binding, configuration specs and configuration declarations choosing architectures.

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

package dh_pkg is
  function clog2(n : positive) return natural;
  function isqrt(n : positive) return positive;
  function n_leaves(g : natural) return positive;
  function leaf_w(g : natural) return positive;
  function leaf_kind(id : natural) return natural;
  function leaf_k(id : natural) return natural;
  function leaf_f(kind : natural; id : natural; d : unsigned) return unsigned;
end package;

package body dh_pkg is

  function clog2(n : positive) return natural is
    variable r : natural := 0;
    variable v : positive := 1;
  begin
    while v < n loop
      v := v * 2;
      r := r + 1;
    end loop;
    return r;
  end function;

  function isqrt(n : positive) return positive is
    variable r : positive := 1;
  begin
    while (r + 1) * (r + 1) <= n loop
      r := r + 1;
    end loop;
    return r;
  end function;

  -- groups differ in leaf count and leaf width
  function n_leaves(g : natural) return positive is
  begin
    return 12 + 4 * (g mod 3);
  end function;

  function leaf_w(g : natural) return positive is
  begin
    return 8 + 8 * ((g + 1) mod 3);
  end function;

  function leaf_kind(id : natural) return natural is
  begin
    return ((id * 37 + id / 5) mod 11) mod 3;
  end function;

  function leaf_k(id : natural) return natural is
  begin
    return (id * 40503 + 12345) mod 16777213;
  end function;

  -- Reference function of one leaf: kind 0 rotate and xor, kind 1 add, kind 2 xor.
  function leaf_f(kind : natural; id : natural; d : unsigned) return unsigned is
    constant W : positive := d'length;
    constant K : unsigned(W - 1 downto 0) := resize(to_unsigned(leaf_k(id), 32), W);
    constant X : unsigned(W - 1 downto 0) := d;
  begin
    case kind is
      when 0      => return rotate_left(X, id mod W) xor K;
      when 1      => return X + K;
      when others => return X xor K;
    end case;
  end function;

end package body;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use work.dh_pkg.all;

-- Sizes computed by functions, used as generics further down.
package dh_cfg is
  constant NREG : positive := clog2(13);
  constant NCL  : positive := isqrt(NREG * 4);
  constant NGRP : positive := isqrt(81);
  constant MAXL : positive := 20;
end package;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use work.dh_pkg.all;

-- Smallest cell: one registered xor with a per-instance constant.
entity dh_cell is
  generic (ID : natural; W : positive);
  port (clk : in std_ulogic;
        d   : in unsigned(W - 1 downto 0);
        q   : out unsigned(W - 1 downto 0) := (others => '0'));
end entity;

architecture rtl_a of dh_cell is
  constant K : unsigned(W - 1 downto 0) := resize(to_unsigned(leaf_k(ID), 32), W);
begin
  process (clk)
  begin
    if rising_edge(clk) then
      q <= d xor K;
    end if;
  end process;
end architecture;

architecture rtl_b of dh_cell is
  constant NK : unsigned(W - 1 downto 0) := not resize(to_unsigned(leaf_k(ID), 32), W);
begin
  -- same function written the other way round
  process (clk)
  begin
    if rising_edge(clk) then
      q <= not ((not d) xor not NK);
    end if;
  end process;
end architecture;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use work.dh_pkg.all;

-- Leaf with three architectures chosen by the instantiating group.
entity dh_leaf is
  generic (ID : natural; W : positive);
  port (clk  : in std_ulogic;
        en   : in std_ulogic := '1';
        d    : in unsigned(W - 1 downto 0);
        q    : out unsigned(W - 1 downto 0) := (others => '0');
        tick : out std_ulogic := '0');
end entity;

architecture rtl_xor of dh_leaf is
  constant K   : unsigned(W - 1 downto 0) := resize(to_unsigned(leaf_k(ID), 32), W);
  constant ROT : natural := ID mod W;
  signal qi    : unsigned(W - 1 downto 0) := (others => '0');
begin
  process (clk)
  begin
    if rising_edge(clk) then
      if en = '1' then
        qi <= rotate_left(d, ROT) xor K;
      end if;
    end if;
  end process;
  q <= qi;
  tick <= qi(0);
end architecture;

architecture rtl_add of dh_leaf is
  constant K : unsigned(W - 1 downto 0) := resize(to_unsigned(leaf_k(ID), 32), W);
  signal qi  : unsigned(W - 1 downto 0) := (others => '0');
begin
  process (clk)
  begin
    if rising_edge(clk) then
      if en = '1' then
        qi <= d + K;
      end if;
    end if;
  end process;
  q <= qi;
  tick <= qi(0);
end architecture;

architecture rtl_cell of dh_leaf is
  component dh_cell is
    generic (ID : natural; W : positive);
    port (clk : in std_ulogic;
          d   : in unsigned(W - 1 downto 0);
          q   : out unsigned(W - 1 downto 0));
  end component;
  signal qi : unsigned(W - 1 downto 0) := (others => '0');
begin
  u_cell : dh_cell
    generic map (ID => ID, W => W)
    port map (clk => clk, d => d, q => qi);
  q <= qi;
  tick <= qi(0);
end architecture;

-- Pick the cell architecture from outside the leaf.
configuration dh_leaf_cell_a of dh_leaf is
  for rtl_cell
    for all : dh_cell
      use entity work.dh_cell(rtl_a);
    end for;
  end for;
end configuration;

configuration dh_leaf_cell_b of dh_leaf is
  for rtl_cell
    for u_cell : dh_cell
      use entity work.dh_cell(rtl_b);
    end for;
  end for;
end configuration;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use work.dh_pkg.all;

-- NL leaves of width LW; their outputs are collected on one bus and xor-reduced.
entity dh_group is
  generic (BASE : natural;
           NL   : positive;
           LW   : positive;
           NWB : positive := NL * LW);
  port (clk : in std_ulogic;
        d   : in std_logic_vector(31 downto 0);
        q   : out std_logic_vector(31 downto 0) := (others => '0');
        dbg : out std_logic_vector(7 downto 0) := (others => '0'));
end entity;

architecture rtl of dh_group is
  component dh_leaf_c is
    generic (ID : natural; W : positive);
    port (clk  : in std_ulogic;
          en   : in std_ulogic := '1';
          d    : in unsigned(W - 1 downto 0);
          q    : out unsigned(W - 1 downto 0);
          tick : out std_ulogic);
  end component;
  signal wbus   : std_logic_vector(NWB - 1 downto 0) := (others => '0');
  signal ticks : std_logic_vector(NL - 1 downto 0) := (others => '0');
begin
  leaves : for l in 0 to NL - 1 generate
    constant ID   : natural := BASE + l;
    constant KIND : natural := leaf_kind(ID);
    subtype slice_t is std_logic_vector(LW - 1 downto 0);
  begin
    k0 : if KIND = 0 generate
      for lf : dh_leaf_c use entity work.dh_leaf(rtl_xor);
    begin
      lf : dh_leaf_c
        generic map (ID => ID, W => LW)
        port map (clk => clk, en => open, d => unsigned(d(LW - 1 downto 0)),
                  std_logic_vector(q) => wbus((l + 1) * LW - 1 downto l * LW),
                  tick => ticks(l));
    end generate;

    k1 : if KIND = 1 generate
      for lf : dh_leaf_c use entity work.dh_leaf(rtl_add);
    begin
      lf : dh_leaf_c
        generic map (ID => ID, W => LW)
        port map (clk => clk, en => '1', d => unsigned(d(LW - 1 downto 0)),
                  std_logic_vector(q) => wbus((l + 1) * LW - 1 downto l * LW),
                  tick => open);
    end generate;

    k2a : if KIND = 2 and ID mod 2 = 0 generate
    begin
      lf : configuration work.dh_leaf_cell_a
        generic map (ID => ID, W => LW)
        port map (clk => clk, en => open, d => unsigned(d(LW - 1 downto 0)),
                  std_logic_vector(q) => wbus((l + 1) * LW - 1 downto l * LW),
                  tick => ticks(l));
    end generate;

    k2b : if KIND = 2 and ID mod 2 = 1 generate
    begin
      lf : configuration work.dh_leaf_cell_b
        generic map (ID => ID, W => LW)
        port map (clk => clk, en => open, d => unsigned(d(LW - 1 downto 0)),
                  std_logic_vector(q) => wbus((l + 1) * LW - 1 downto l * LW),
                  tick => ticks(l));
    end generate;
  end generate;

  red : process (clk)
    variable acc : std_logic_vector(31 downto 0);
  begin
    if rising_edge(clk) then
      acc := (others => '0');
      for l in 0 to NL - 1 loop
        acc := acc xor std_logic_vector(resize(unsigned(wbus((l + 1) * LW - 1 downto l * LW)), 32));
      end loop;
      q <= acc;
    end if;
  end process;

  cnt : process (clk)
    variable n : natural;
  begin
    if rising_edge(clk) then
      n := 0;
      for l in ticks'range loop
        if ticks(l) = '1' then
          n := n + 1;
        end if;
      end loop;
      dbg <= std_logic_vector(to_unsigned(n, 8));
    end if;
  end process;
end architecture;

configuration dh_group_cfg of dh_group is
  for rtl
  end for;
end configuration;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use work.dh_pkg.all;
use work.dh_cfg.all;

entity dh_cluster is
  generic (BASE : natural);
  port (clk : in std_ulogic;
        d   : in std_logic_vector(31 downto 0);
        q   : out std_logic_vector(31 downto 0) := (others => '0');
        dbg : out std_logic_vector(7 downto 0) := (others => '0'));
end entity;

architecture rtl of dh_cluster is
  component dh_group_c is
    generic (BASE : natural; NL : positive; LW : positive; NWB : positive := NL * LW);
    port (clk : in std_ulogic;
          d   : in std_logic_vector(31 downto 0);
          q   : out std_logic_vector(31 downto 0);
          dbg : out std_logic_vector(7 downto 0));
  end component;
  type gq_t is array (0 to NGRP - 1) of std_logic_vector(31 downto 0);
  signal gq   : gq_t := (others => (others => '0'));
  signal gdbg : std_logic_vector(NGRP * 8 - 1 downto 0) := (others => '0');
begin
  grps : for g in 0 to NGRP - 1 generate
    constant NL : positive := n_leaves(g);
    constant LW : positive := leaf_w(g);
    constant GB : natural := BASE + g * MAXL;
  begin
    v0 : if g mod 3 = 0 generate
      for ug : dh_group_c use entity work.dh_group(rtl);
    begin
      ug : dh_group_c
        generic map (BASE => GB, NL => NL, LW => LW)
        port map (clk => clk, d => d, q => gq(g), dbg => gdbg(g * 8 + 7 downto g * 8));
    end generate;

    v1 : if g mod 3 = 1 generate
    begin
      ug : entity work.dh_group
        generic map (BASE => GB, NL => NL, LW => LW)
        port map (clk => clk, d => d, q => gq(g), dbg => open);
    end generate;

    v2 : if g mod 3 = 2 generate
    begin
      ug : configuration work.dh_group_cfg
        generic map (BASE => GB, NL => NL, LW => LW)
        port map (clk => clk, d => d, q => gq(g), dbg => gdbg(g * 8 + 7 downto g * 8));
    end generate;
  end generate;

  red : process (clk)
    variable acc : std_logic_vector(31 downto 0);
    variable dsum : std_logic_vector(7 downto 0);
  begin
    if rising_edge(clk) then
      acc := (others => '0');
      dsum := (others => '0');
      for g in gq'range loop
        acc := acc xor gq(g);
        dsum := dsum xor gdbg(g * 8 + 7 downto g * 8);
      end loop;
      q <= acc;
      dbg <= dsum;
    end if;
  end process;
end architecture;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use work.dh_pkg.all;
use work.dh_cfg.all;

entity dh_region is
  generic (R : natural);
  port (clk : in std_ulogic;
        d   : in std_logic_vector(31 downto 0);
        q   : out std_logic_vector(31 downto 0) := (others => '0');
        dbg : out std_logic_vector(7 downto 0) := (others => '0'));
end entity;

architecture rtl of dh_region is
  -- no configuration spec: default binding to the entity of the same name
  component dh_cluster is
    generic (BASE : natural);
    port (clk : in std_ulogic;
          d   : in std_logic_vector(31 downto 0);
          q   : out std_logic_vector(31 downto 0);
          dbg : out std_logic_vector(7 downto 0));
  end component;
begin
  rb : block is
    constant RBASE : natural := R * NCL * NGRP * MAXL;
    type cq_t is array (0 to NCL - 1) of std_logic_vector(31 downto 0);
    signal cq   : cq_t := (others => (others => '0'));
    signal cdbg : std_logic_vector(NCL * 8 - 1 downto 0) := (others => '0');
  begin
    clusters : for c in 0 to NCL - 1 generate
      constant CB : natural := RBASE + c * NGRP * MAXL;
    begin
      uc : dh_cluster
        generic map (BASE => CB)
        port map (clk => clk, d => d, q => cq(c), dbg => cdbg(c * 8 + 7 downto c * 8));
    end generate;

    red : process (clk)
      variable acc  : std_logic_vector(31 downto 0);
      variable dsum : std_logic_vector(7 downto 0);
    begin
      if rising_edge(clk) then
        acc := (others => '0');
        dsum := (others => '0');
        for c in cq'range loop
          acc := acc xor cq(c);
          dsum := dsum xor cdbg(c * 8 + 7 downto c * 8);
        end loop;
        q <= acc;
        dbg <= dsum;
      end if;
    end process;
  end block;
end architecture;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use work.dh_pkg.all;

entity dh_top is
  generic (N : positive);
  port (clk : in std_ulogic;
        din : in unsigned(31 downto 0);
        q   : out std_logic_vector(31 downto 0) := (others => '0');
        dbg : out std_logic_vector(7 downto 0) := (others => '0'));
end entity;

architecture rtl of dh_top is
  type rq_t is array (0 to N - 1) of std_logic_vector(31 downto 0);
  signal rq   : rq_t := (others => (others => '0'));
  signal rdbg : std_logic_vector(N * 8 - 1 downto 0) := (others => '0');
begin
  regs : for r in 0 to N - 1 generate
  begin
    r_even : if r mod 2 = 0 generate
    begin
      ur : entity work.dh_region
        generic map (R => r)
        port map (clk => clk, d => std_logic_vector(din), q => rq(r),
                  dbg => rdbg(r * 8 + 7 downto r * 8));
    end generate;

    r_odd : if r mod 2 = 1 generate
    begin
      ur : entity work.dh_region
        generic map (R => r)
        port map (clk => clk, d => std_logic_vector(din), q => rq(r), dbg => open);
    end generate;
  end generate;

  red : process (clk)
    variable acc  : std_logic_vector(31 downto 0);
    variable dsum : std_logic_vector(7 downto 0);
  begin
    if rising_edge(clk) then
      acc := (others => '0');
      dsum := (others => '0');
      for r in rq'range loop
        acc := acc xor rq(r);
        dsum := dsum xor rdbg(r * 8 + 7 downto r * 8);
      end loop;
      q <= acc;
      dbg <= dsum;
    end if;
  end process;
end architecture;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use work.dh_pkg.all;
use work.dh_cfg.all;

entity deep_hier_tb is
end entity;

architecture sim of deep_hier_tb is
  constant CYCLES : natural := 64;
  constant LATENCY : natural := 4;
  -- expected rolling checksum of top output and debug word (from the interpreter run)
  constant GOLD : unsigned(31 downto 0) := x"517AE920";

  signal clk  : std_ulogic := '0';
  signal din  : std_logic_vector(31 downto 0) := (others => '0');
  signal qtop : std_logic_vector(31 downto 0);
  signal dbg  : std_logic_vector(7 downto 0);

  type hist_t is array (0 to LATENCY) of std_logic_vector(31 downto 0);

  -- Expected top output for one input word: every leaf evaluated by the reference function.
  function model_out(d : std_logic_vector(31 downto 0)) return std_logic_vector is
    variable acc : unsigned(31 downto 0) := (others => '0');
    variable id  : natural;
    variable lw  : positive;
  begin
    for r in 0 to NREG - 1 loop
      for c in 0 to NCL - 1 loop
        for g in 0 to NGRP - 1 loop
          lw := leaf_w(g);
          for l in 0 to n_leaves(g) - 1 loop
            id := ((r * NCL + c) * NGRP + g) * MAXL + l;
            acc := acc xor resize(leaf_f(leaf_kind(id), id, unsigned(d(lw - 1 downto 0))), 32);
          end loop;
        end loop;
      end loop;
    end loop;
    return std_logic_vector(acc);
  end function;
begin
  clk <= not clk after 5 ns;

  dut : entity work.dh_top
    generic map (N => NREG)
    port map (clk => clk, din => unsigned(din), q => qtop, dbg => dbg);

  stim : process
    variable lfsr : unsigned(31 downto 0) := x"ACE1BEEF";
    variable hist : hist_t := (others => (others => '0'));
    variable acc  : unsigned(31 downto 0) := (others => '0');
    variable checks : natural := 0;
  begin
    for n in 0 to CYCLES - 1 loop
      -- xorshift word, applied before edge n
      lfsr := lfsr xor shift_left(lfsr, 13);
      lfsr := lfsr xor shift_right(lfsr, 17);
      lfsr := lfsr xor shift_left(lfsr, 5);
      for k in LATENCY downto 1 loop
        hist(k) := hist(k - 1);
      end loop;
      hist(0) := std_logic_vector(lfsr);
      din <= hist(0);

      wait until rising_edge(clk);
      wait until falling_edge(clk);
      if n >= LATENCY and n mod 4 = 0 then
        assert qtop = model_out(hist(LATENCY))
          report "top output mismatch at cycle " & integer'image(n) severity failure;
        checks := checks + 1;
      end if;
      acc := rotate_left(acc, 1) xor unsigned(qtop) xor resize(unsigned(dbg), 32);
    end loop;

    assert checks > 8 report "too few model checks" severity failure;
    assert acc = GOLD report "rolling checksum mismatch" severity failure;
    report "PASS" severity note;
    wait;
  end process;
end architecture;
