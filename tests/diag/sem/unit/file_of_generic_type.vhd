-- yavs-test: std=2019
-- expect: E1201 6:12 a file declaration needs a file type
-- A generic file type is a file type, another class is not (6.5.3, #915)
package p is
  generic (type t is private);
  file f : t;
end package;
