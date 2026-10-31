-- lint_790: a signal declared in a wider region than the one that uses it.
entity scope is
  generic (en : boolean := true);
  port (q : in bit_vector(3 downto 0); y : out bit);
end entity;

architecture rtl of scope is
  signal lane_hit : bit; -- expect: lint_790
  signal in_block : bit; -- expect: lint_790
  signal in_alt : bit; -- expect: lint_790
  signal deep_one : bit; -- expect: lint_790
  signal in_process : bit; -- expect: lint_790
  signal two_regions : bit;
  signal in_statement_part : bit;
  signal in_guard : bit;
  signal in_a_map : bit;
  signal unused_here : bit;
  signal kept : bit;
  attribute mark : string;
  attribute mark of kept : signal is "x";
  constant width : natural := 4;
  signal in_a_range : bit_vector(width - 1 downto 0);
begin
  y <= in_statement_part;

  g_lanes : for i in 0 to 3 generate
    lane_hit <= q(i);
  end generate;

  b : block is
  begin
    in_block <= q(0);
  end block;

  g_on : if en generate
    in_alt <= q(0);
    two_regions <= q(0);
  else generate
    two_regions <= q(1);
  end generate;

  g_outer : if en generate
    signal moves_further : bit; -- expect: lint_790
  begin
    g_inner : if en generate
      deep_one <= q(0);
      moves_further <= q(1);
    end generate;
  end generate;

  g_proc : if en generate
    process (q) is
    begin
      in_process <= q(0);
    end process;
  end generate;

  -- Read in the region around the block, so they stay though the block assigns them.
  g_block : block (in_guard = '1') is
    port (p : in bit);
    port map (p => in_a_map);
  begin
    assert p = '1';
    in_guard <= q(2);
    in_a_map <= q(3);
  end block;

  g_kept : if en generate
    kept <= q(0);
  end generate;

  g_range : for i in in_a_range'range generate
    assert q(i) = '0';
  end generate;
end architecture;
