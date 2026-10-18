-- yavs-test: std=2008
-- expect: E1503 20:43 not an instance of mux
-- LRM 6.5.7.2: the actual of a formal package is an instance of its generic package
package mux is
  generic (size : natural);
  type data_array is array (natural range <>) of bit_vector(size - 1 downto 0);
end package;
package other is
  generic (size : natural);
  constant c : natural := size;
end package;
entity m is
  generic (package mux_g is new work.mux generic map (<>));
end entity;
entity t is
end entity;
architecture a of t is
  package other_i is new work.other generic map (size => 2);
begin
  u : entity work.m generic map (mux_g => other_i);
end architecture;
