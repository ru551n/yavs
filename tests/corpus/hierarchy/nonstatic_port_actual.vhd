-- difftest: top=tb_nonstatic_port_actual std=2008
-- A signal name with a subscript that is not static as the actual of an `in` port, of a part and
-- under a type conversion: an expression actual, driven one delta after any signal it reads.
entity sub_nspa is
  port (d : in bit_vector(1 downto 0); o : out bit_vector(1 downto 0); n : in natural range 0 to 9 := 0);
end entity;
architecture a of sub_nspa is
begin
  o <= d;
  process (n) begin report "n=" & integer'image(n); end process;
end architecture;

entity conv_nspa is
  port (d : in integer);
end entity;
architecture a of conv_nspa is
begin
  process (d) begin report "d=" & integer'image(d); end process;
end architecture;

entity tb_nonstatic_port_actual is end entity;
architecture a of tb_nonstatic_port_actual is
  type arr is array (0 to 1) of bit_vector(3 downto 0);
  type iv is array (0 to 1) of natural;
  type rv is array (0 to 1) of real;
  signal q : arr := ("0001", "0010");
  signal r : natural range 0 to 1 := 0;
  signal o, o2 : bit_vector(1 downto 0);
  signal t : iv := (3, 4);
  signal x : rv := (1.0, 2.0);
  component c is
    port (d : in bit_vector(1 downto 0); o : out bit_vector(1 downto 0); n : in natural range 0 to 9 := 0);
  end component;
  for u2 : c use entity work.sub_nspa port map (d => d, o => o, n => n);
begin
  u1 : entity work.sub_nspa port map (d => q(r)(1 downto 0), o => o, n => t(r));
  u2 : c port map (d(1) => q(r)(0), d(0) => q(1 - r)(1), o => o2, n => 1);
  u3 : entity work.conv_nspa port map (d => integer(x(r)));
  process begin
    wait for 1 ns;
    report to_string(o) & " " & to_string(o2);
    r <= 1;
    wait for 1 ns;
    report to_string(o) & " " & to_string(o2);
    q(1) <= "0011";
    wait for 1 ns;
    report to_string(o) & " " & to_string(o2);
    t(1) <= 7;
    wait for 1 ns;
    wait;
  end process;
end architecture;
