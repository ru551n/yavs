-- difftest: top=unelaborated_generate_constant std=2008
-- A constant declared in a generate branch that is not elaborated is never evaluated
-- (#495): `bpb` would be 0, outside `positive`, when `user_width` is 0. The architecture
-- has a constant with a non-static value, so its constants get per-instance thunks.
entity unelaborated_generate_constant_sub is
  generic (user_width : natural := 0; sfx : string := "");
end entity;

architecture a of unelaborated_generate_constant_sub is
  constant k : string := "abc" & sfx;
begin
  g : if user_width > 0 generate
    constant bpb : positive := user_width / 8;
  begin
    process
    begin
      report "bytes " & integer'image(bpb);
      wait;
    end process;
  end generate;
  process
  begin
    report "k=" & k & " width=" & integer'image(user_width);
    wait;
  end process;
end architecture;

entity unelaborated_generate_constant is
end entity;

architecture t of unelaborated_generate_constant is
begin
  u0 : entity work.unelaborated_generate_constant_sub;
  u1 : entity work.unelaborated_generate_constant_sub generic map (user_width => 16, sfx => "x");
  g : for i in 0 to 1 generate
    constant w : natural := i * 8;
  begin
    u : entity work.unelaborated_generate_constant_sub generic map (user_width => w);
  end generate;
end architecture;
