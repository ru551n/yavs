-- LRM 9.2.3: the ordering operators on two null arrays. A null array is less only than a
-- non-null array, so "" < "" is FALSE and "" <= "" is TRUE.
entity top is end entity;
architecture a of top is
begin
  process
    variable e : string(1 to 0);
    variable b : bit_vector(3 downto 4);
    variable i : integer_vector(1 to 0);
  begin
    report "RES str lt=" & boolean'image(e < "") & " le=" & boolean'image(e <= "")
      & " gt=" & boolean'image(e > "") & " ge=" & boolean'image(e >= "");
    report "RES bv lt=" & boolean'image(b < b) & " ge=" & boolean'image(b >= b);
    report "RES intv lt=" & boolean'image(i < i) & " ge=" & boolean'image(i >= i);
    report "RES null lt nonnull=" & boolean'image(e < "a");
    wait;
  end process;
end architecture;
