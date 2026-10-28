-- yavs-test: std=2008
-- expect: E1001 5:9 library `iee` does not exist
-- help: a library with a similar name exists: `ieee`
-- LRM 13.2
library iee;
package q is
end package;
