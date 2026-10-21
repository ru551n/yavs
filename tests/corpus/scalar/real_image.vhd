-- difftest: top=real_image std=2008
-- real'image text is implementation-defined; sim1's shortest-round-trip form is the reference.
entity real_image is end;
architecture t of real_image is
begin
  process begin
    report "pi=" & real'image(3.14159) & " big=" & real'image(1.0e10) & " small=" & real'image(1.5e-5);
    report "one=" & real'image(1.0) & " half=" & real'image(0.5) & " neg=" & real'image(-2.25);
    report "mul=" & real'image(10.0 * 0.1);
    report "to_string=" & to_string(2.5);
    wait;
  end process;
end;
