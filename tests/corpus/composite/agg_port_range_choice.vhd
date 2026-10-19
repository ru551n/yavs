-- difftest: top=agg_port_range_choice std=2008
-- Aggregates whose choice is the range of an unconstrained port (#311), in signal and
-- constant initial values of the instance: a range choice, its reverse range, a subtype
-- of the port's range as choice and an in port's range.
entity agg_port_range_choice_m is
  port (
    tid  : out bit_vector;
    data : in  bit_vector
  );
end entity;

architecture a of agg_port_range_choice_m is
  subtype tid_r is integer range tid'range;
  signal pid   : bit_vector(tid'range) := (tid'range => '0');
  signal ones  : bit_vector(tid'range) := (tid'reverse_range => '1');
  constant sub : bit_vector(tid'range) := (tid_r => '1');
  constant mix : bit_vector(data'range) := (data'range => '1');
  constant hl  : integer := tid_r'high - tid_r'low;
begin
  process
  begin
    report "p " & integer'image(pid'length) & " " & to_string(pid) & " " & to_string(ones)
      & " " & to_string(sub) & " " & to_string(mix) & " " & integer'image(hl);
    wait;
  end process;
end architecture;

entity agg_port_range_choice is
end entity;

architecture t of agg_port_range_choice is
  signal s : bit_vector(3 downto 0);
  signal d : bit_vector(1 to 5) := "10101";
begin
  m : entity work.agg_port_range_choice_m port map (tid => s, data => d);
end architecture;
