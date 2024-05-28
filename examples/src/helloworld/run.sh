cargo run --release --bin helloworld-client-bench -- \
	--rps 100 \
	--secs 10 \
	--concurrency 32 \
	--addr1 http://[::1]:50051 \
	--addr2 http://[::1]:50052 \
	--output1 client1.txt \
	--output2 client2.txt
