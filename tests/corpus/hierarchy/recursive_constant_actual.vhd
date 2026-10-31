-- difftest: top=recursive_constant_actual std=2008
-- A recursive design whose generic actual is a constant of the instance, computed by an
-- operator or a function: every level has its own value (#913).
entity rec_rca is
  generic (n : positive);
end entity;
architecture a of rec_rca is
  function split (x : natural) return natural is begin return x / 4; end function;
  constant nxt : natural := n / 4;
  constant nx2 : natural := split(n);
  type rv is array (0 to 1) of natural;
  constant t : rv := (nxt, nx2);
  component rec_rca is generic (n : positive); end component;
begin
  more : if t(0) > 1 generate
    constant here : natural := t(1);
  begin
    next_inst : rec_rca generic map (n => here);
  end generate;
  process begin
    report "n = " & integer'image(n) & " nxt = " & integer'image(nxt) & " " & integer'image(t(1));
    wait;
  end process;
end architecture;

entity recursive_constant_actual is end entity;
architecture a of recursive_constant_actual is begin
  dut : entity work.rec_rca generic map (n => 28);
end architecture;
