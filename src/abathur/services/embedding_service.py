"""Embedding generation service using Ollama."""

import time

import httpx


class EmbeddingService:
    """Service for generating embeddings using Ollama.

    Uses the nomic-embed-text-v1.5 model which produces 768-dimensional
    embeddings for semantic search capabilities.
    """

    def __init__(self, base_url: str = "http://localhost:11434"):
        """Initialize embedding service.

        Args:
            base_url: Ollama API base URL
        """
        self.base_url = base_url
        self.model = "nomic-embed-text"

    async def generate_embedding(self, text: str) -> list[float]:
        """Generate embedding vector for text.

        Args:
            text: Input text to embed

        Returns:
            768-dimensional embedding vector

        Raises:
            httpx.HTTPError: If Ollama API request fails
            ValueError: If embedding dimensions are incorrect

        Example:
            >>> service = EmbeddingService()
            >>> embedding = await service.generate_embedding("Hello, world!")
            >>> len(embedding)  # 768
        """
        start_time = time.time()

        async with httpx.AsyncClient(timeout=30.0) as client:
            response = await client.post(
                f"{self.base_url}/api/embeddings",
                json={"model": self.model, "prompt": text},
            )
            response.raise_for_status()
            data = response.json()
            embedding: list[float] = data["embedding"]

            # Validate dimensions
            if len(embedding) != 768:
                raise ValueError(f"Expected 768 dimensions, got {len(embedding)}")

            elapsed_ms = (time.time() - start_time) * 1000
            print(f"Generated embedding in {elapsed_ms:.2f}ms")

            return embedding

    async def generate_batch(self, texts: list[str]) -> list[list[float]]:
        """Generate embeddings for multiple texts.

        Args:
            texts: List of input texts

        Returns:
            List of 768-dimensional embedding vectors

        Example:
            >>> service = EmbeddingService()
            >>> texts = ["text1", "text2", "text3"]
            >>> embeddings = await service.generate_batch(texts)
            >>> len(embeddings)  # 3
        """
        embeddings = []
        for i, text in enumerate(texts):
            embedding = await self.generate_embedding(text)
            embeddings.append(embedding)
            print(f"Batch progress: {i + 1}/{len(texts)}")
        return embeddings

    async def health_check(self) -> bool:
        """Check if Ollama service is accessible.

        Returns:
            True if service is healthy, False otherwise

        Example:
            >>> service = EmbeddingService()
            >>> is_healthy = await service.health_check()
        """
        try:
            async with httpx.AsyncClient(timeout=5.0) as client:
                response = await client.get(f"{self.base_url}/api/tags")
                return bool(response.status_code == 200)
        except Exception:
            return False
