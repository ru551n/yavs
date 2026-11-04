-- Instances of a process that differ in values it only computes with (a coefficient, a position
-- that picks a branch) and in a width: `tests/spec.rs` runs it on copies that share a body and
-- requires what the shared code prints. The sum is the check.
entity share_pe is
  generic (ROW, COL, W, COEF : integer);
  port (clk : in bit; dout : out integer);
end entity;

architecture a of share_pe is
  signal r0 : bit_vector(W - 1 downto 0) := (others => '1');
begin
  mac : process (clk)
    variable n : integer := ROW;
  begin
    if clk = '1' then
      r0 <= r0(W - 2 downto 0) & r0(W - 1);
      if (ROW + COL) mod 2 = 0 then
        n := (n + COEF) mod 4099;
      else
        n := (n * 3 + COL) mod 4099;
      end if;
      if r0(COL mod W) = '1' then
        n := (n + 1) mod 4099;
      end if;
      dout <= n + ROW;
    end if;
  end process;
end architecture;

entity share_values is
end entity;

architecture t of share_values is
  constant N : integer := 4;
  signal clk : bit := '0';
  type outs_t is array (0 to N * N - 1) of integer;
  signal outs : outs_t;
begin
  clk <= not clk after 5 ns;
  gi : for i in 0 to N - 1 generate
    gj : for j in 0 to N - 1 generate
      u : entity work.share_pe
        generic map (ROW => i * 2, COL => j, W => 6 + (i mod 2), COEF => 3 + i * 131 + j * 17)
        port map (clk, outs(i * N + j));
    end generate;
  end generate;
  process
    variable s : integer := 0;
  begin
    wait for 2 us;
    for k in outs'range loop
      s := s + outs(k);
    end loop;
    report "sum=" & integer'image(s);
    std.env.finish;
  end process;
end architecture;
