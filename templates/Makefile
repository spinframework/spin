CRATES_IO_VERSION := $(shell echo "${SDK_VERSION}" | tr -d 'v')

bump-versions: bump-go-versions bump-rust-versions

bump-go-versions:
	@for dir in $$(ls -d *-go) ; do \
		cd $$dir/content ; \
		sed -i.sed-bak -e 's/{{project-name | snake_case}}/foo/g' go.mod ; \
		go get -u github.com/spinframework/spin-go-sdk ; \
		go mod tidy ; \
		sed -i.sed-bak -e 's/foo/{{project-name | snake_case}}/g' go.mod ; \
		rm *.sed-bak ; \
		cd - 2>&1 >/dev/null ; \
	done

bump-rust-versions:
	for dir in $$(ls -d *-rust) ; do \
		cd $$dir/content ; \
		sed -r -i.sed-bak -e 's%(spin-sdk = )(("[0-9]+.[0-9]+.[0-9]+(-rc.[0-9]+)?")|(\{ git = "https://github.com/spinframework/spin", branch = "main" \}))%spin-sdk = "${CRATES_IO_VERSION}"%g' Cargo.toml.tmpl ; \
		rm *.sed-bak ; \
		cd - 2>&1 >/dev/null ; \
	done
