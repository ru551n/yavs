-- yavs-test: std=2008
-- expect: none
-- LRM 6.5.6.3, 6.5.7.2: a design entity's formal package takes a package instance; its types are the instance's in the ports (#173)
package mux is
  generic (size : natural);
  subtype data is bit_vector(size - 1 downto 0);
  type data_array is array (natural range <>) of data;
end package;
entity m is
  generic (size : natural; package mux_g is new work.mux generic map (size => size));
  port (i : in mux_g.data_array(0 to 3); o : out mux_g.data);
end entity;
architecture a of m is
begin
  o <= i(0);
end architecture;
entity t is
end entity;
architecture a of t is
  package mux_i is new work.mux generic map (size => 2);
  signal i : mux_i.data_array(0 to 3);
  signal o : mux_i.data;
begin
  u : entity work.m generic map (size => 2, mux_g => mux_i) port map (i => i, o => o);
end architecture;
